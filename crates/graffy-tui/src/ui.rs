//! Terminal rendering + event loops (ADR-0002, Phase 1 M3).
//!
//! Three entry points:
//! * [`run_live`] — execute a graph while rendering the journal tap in real
//!   time; stays open for inspection after the run finishes.
//! * [`run_replay`] — fold a finished journal and open the step inspector.
//! * [`run_home`] — pick a journal from `graffy-runs/` and inspect it.
//!
//! Keys: `q`/`Esc` quit · `Tab` switch Run/Inspect · `↑`/`↓` select node ·
//! `j`/`k` (or PgDn/PgUp) scroll detail · approvals are auto-logged in M3
//! (interactive approval prompts arrive with M4).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use graffy_core::error::ExecError;
use graffy_core::exec::{AutoApprove, Executor, ModelInvoker, RunInput, RunOutcome};
use graffy_core::journal::{JournalReader, wire};
use graffy_core::spec::GraphSpec;

use crate::state::{AppState, state_glyph, state_word, status_word};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Run,
    Inspect,
}

struct UiState {
    view: View,
    node_list: ListState,
    detail_scroll: u16,
}

impl UiState {
    fn new(view: View) -> Self {
        Self {
            view,
            node_list: ListState::default(),
            detail_scroll: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Execute a graph live inside the TUI. Returns the run outcome (None if the
/// executor task failed; its error is printed after terminal restore).
pub async fn run_live(
    spec: GraphSpec,
    spec_toml: String,
    prompt: String,
    journal_path: PathBuf,
    invoker: Arc<dyn ModelInvoker>,
) -> Result<Option<RunOutcome>> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let executor = Executor {
        event_tap: Some(tx),
        ..Default::default()
    };

    let spec_for_task = spec.clone();
    let path_for_task = journal_path.clone();
    let task: tokio::task::JoinHandle<Result<RunOutcome, ExecError>> = tokio::spawn(async move {
        let approvals = AutoApprove;
        executor
            .run(
                &spec_for_task,
                &spec_toml,
                RunInput {
                    prompt,
                    session_id: None,
                },
                &path_for_task,
                invoker.as_ref(),
                &approvals,
            )
            .await
    });

    let mut app = AppState::default();
    app.seed_from_spec(&spec);
    let mut ui = UiState::new(View::Run);

    let mut terminal = ratatui::init();
    let loop_result = live_loop(&mut terminal, &mut rx, &task, &mut app, &mut ui);
    ratatui::restore();
    loop_result?;

    if !task.is_finished() {
        eprintln!(
            "(quit before completion — letting the run finish headless so the journal stays whole)"
        );
    }
    match task.await {
        Ok(Ok(outcome)) => Ok(Some(outcome)),
        Ok(Err(err)) => {
            eprintln!("run error: {err}");
            Ok(None)
        }
        Err(join_err) => {
            eprintln!("executor task panicked: {join_err}");
            Ok(None)
        }
    }
}

/// Fold a finished journal and open the inspector.
pub fn run_replay(journal_path: &Path) -> Result<()> {
    let events = JournalReader::read_all(journal_path)?;
    let mut app = AppState::default();
    for frame in &events {
        app.apply(frame);
    }
    let mut ui = UiState::new(View::Inspect);
    let mut terminal = ratatui::init();
    let result = static_loop(&mut terminal, &app, &mut ui);
    ratatui::restore();
    result
}

/// Journal picker over `graffy-runs/`, opening the inspector on selection.
pub fn run_home() -> Result<()> {
    let dir = Path::new("graffy-runs");
    let mut journals: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "journal"))
                .collect()
        })
        .unwrap_or_default();
    // ULID-named files sort chronologically; newest first.
    journals.sort();
    journals.reverse();

    if journals.is_empty() {
        println!("no journals in graffy-runs/ yet.");
        println!(
            "try: graffy run graphs/conversation.default.toml --prompt \"hello\" --offline --tui"
        );
        return Ok(());
    }

    loop {
        let Some(choice) = pick_journal(&journals)? else {
            return Ok(());
        };
        run_replay(&choice)?;
    }
}

fn pick_journal(journals: &[PathBuf]) -> Result<Option<PathBuf>> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));
    let mut terminal = ratatui::init();
    let result = (|| -> Result<Option<PathBuf>> {
        loop {
            terminal.draw(|f| {
                let [header, body] =
                    Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).areas(f.area());
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" graffy ", Style::new().add_modifier(Modifier::BOLD)),
                        Span::raw("· pick a run journal — Enter inspect · q quit"),
                    ])),
                    header,
                );
                let items: Vec<ListItem> = journals
                    .iter()
                    .map(|p| {
                        ListItem::new(
                            p.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        )
                    })
                    .collect();
                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" graffy-runs/ "),
                    )
                    .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
                f.render_stateful_widget(list, body, &mut list_state);
            })?;
            if event::poll(Duration::from_millis(100))?
                && let TermEvent::Key(key) = event::read()?
            {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Up => select_prev(&mut list_state, journals.len()),
                    KeyCode::Down => select_next(&mut list_state, journals.len()),
                    KeyCode::Enter => {
                        let ix = list_state.selected().unwrap_or(0);
                        return Ok(journals.get(ix).cloned());
                    }
                    _ => {}
                }
            }
        }
    })();
    ratatui::restore();
    result
}

// ---------------------------------------------------------------------------
// Loops
// ---------------------------------------------------------------------------

fn live_loop(
    terminal: &mut DefaultTerminal,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<wire::RunEvent>,
    task: &tokio::task::JoinHandle<Result<RunOutcome, ExecError>>,
    app: &mut AppState,
    ui: &mut UiState,
) -> Result<()> {
    loop {
        while let Ok(frame) = rx.try_recv() {
            app.apply(&frame);
        }
        let finished = task.is_finished() && app.status.is_some();
        terminal.draw(|f| draw(f, app, ui, finished))?;
        if event::poll(Duration::from_millis(50))?
            && let TermEvent::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && handle_key(key.code, app, ui)
        {
            return Ok(());
        }
    }
}

fn static_loop(terminal: &mut DefaultTerminal, app: &AppState, ui: &mut UiState) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app, ui, true))?;
        if event::poll(Duration::from_millis(100))?
            && let TermEvent::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && handle_key(key.code, app, ui)
        {
            return Ok(());
        }
    }
}

/// Returns true when the user asked to quit.
fn handle_key(code: KeyCode, app: &AppState, ui: &mut UiState) -> bool {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Tab => {
            ui.view = match ui.view {
                View::Run => View::Inspect,
                View::Inspect => View::Run,
            };
        }
        KeyCode::Up => {
            select_prev(&mut ui.node_list, app.nodes.len());
            ui.detail_scroll = 0;
        }
        KeyCode::Down => {
            select_next(&mut ui.node_list, app.nodes.len());
            ui.detail_scroll = 0;
        }
        KeyCode::Char('j') | KeyCode::PageDown => {
            ui.detail_scroll = ui.detail_scroll.saturating_add(3);
        }
        KeyCode::Char('k') | KeyCode::PageUp => {
            ui.detail_scroll = ui.detail_scroll.saturating_sub(3);
        }
        _ => {}
    }
    false
}

fn select_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let next = match state.selected() {
        Some(0) | None => len - 1,
        Some(i) => i - 1,
    };
    state.select(Some(next));
}

fn select_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let next = match state.selected() {
        Some(i) if i + 1 < len => i + 1,
        _ => 0,
    };
    state.select(Some(next));
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw(f: &mut Frame, app: &AppState, ui: &mut UiState, finished: bool) {
    if ui.node_list.selected().is_none() && !app.nodes.is_empty() {
        ui.node_list.select(Some(0));
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .areas(f.area());

    // Header: identity + MCW counters.
    let status_style = match app.status {
        Some(wire::RunStatus::Succeeded) => {
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
        }
        Some(wire::RunStatus::Failed) | Some(wire::RunStatus::BudgetExhausted) => {
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
        }
        Some(wire::RunStatus::Cancelled) => Style::new().fg(Color::Magenta),
        _ => Style::new().fg(Color::Cyan),
    };
    let title = Line::from(vec![
        Span::styled(" graffy ", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(format!("▸ {} v{} ", app.graph_name, app.graph_version)),
        Span::styled(format!("[{}]", status_word(app.status)), status_style),
        Span::styled(
            format!("  {}", app.run_id),
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    let counters = Line::from(vec![
        Span::raw(format!(
            " IU {} · evidence {} · model calls {} · routing {}",
            app.iu_count, app.evidence_count, app.model_calls, app.routing_decisions
        )),
        Span::styled(
            format!(
                " · failures {} · repairs {}",
                app.failure_count, app.repair_count
            ),
            if app.failure_count > 0 {
                Style::new().fg(Color::Yellow)
            } else {
                Style::new().fg(Color::DarkGray)
            },
        ),
        Span::raw(format!(
            " · {}⇢{} tok · ${:.4}",
            app.input_tokens, app.output_tokens, app.total_usd
        )),
        if app.max_escalation > 0 {
            Span::styled(
                format!(" · escalated ×{}", app.max_escalation),
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        },
    ]);
    f.render_widget(Paragraph::new(vec![title, counters]), header);

    // Body.
    match ui.view {
        View::Run => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
                    .areas(body);
            f.render_stateful_widget(node_list(app), left, &mut ui.node_list);

            let feed_height = right.height.saturating_sub(2) as usize;
            let lines: Vec<Line> = app
                .feed
                .iter()
                .rev()
                .take(feed_height)
                .rev()
                .map(|l| {
                    Line::from(vec![
                        Span::styled(format!("#{:<4} ", l.seq), Style::new().fg(Color::DarkGray)),
                        Span::raw(l.text.clone()),
                    ])
                })
                .collect();
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" journal feed (live) "),
                ),
                right,
            );
        }
        View::Inspect => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .areas(body);
            f.render_stateful_widget(node_list(app), left, &mut ui.node_list);

            let selected = ui
                .node_list
                .selected()
                .and_then(|i| app.nodes.get(i))
                .map(|row| row.id.clone())
                .unwrap_or_default();
            let mut lines: Vec<Line> = Vec::new();
            match app.inspector.get(&selected) {
                Some(entries) => {
                    for entry in entries {
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("#{:<4} ", entry.seq),
                                Style::new().fg(Color::DarkGray),
                            ),
                            Span::styled(
                                entry.title.clone(),
                                Style::new().add_modifier(Modifier::BOLD),
                            ),
                        ]));
                        for body_line in &entry.body {
                            lines.push(Line::from(format!("      {body_line}")));
                        }
                        lines.push(Line::from(""));
                    }
                }
                None => lines.push(Line::from("no recorded steps for this node yet")),
            }
            f.render_widget(
                Paragraph::new(lines)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" inspect: {selected} ")),
                    )
                    .wrap(Wrap { trim: false })
                    .scroll((ui.detail_scroll, 0)),
                right,
            );
        }
    }

    // Footer: novice strip + keys.
    let novice = Line::from(Span::styled(
        format!(" {}", app.novice_line),
        Style::new()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    ));
    let keys = Line::from(Span::styled(
        if finished {
            " q quit · Tab run/inspect · ↑↓ node · j/k scroll — run finished, journal on disk"
        } else {
            " q quit · Tab run/inspect · ↑↓ node · j/k scroll"
        },
        Style::new().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(vec![novice, keys]), footer);
}

fn node_list(app: &AppState) -> List<'static> {
    let items: Vec<ListItem> = app
        .nodes
        .iter()
        .map(|row| {
            let style = node_style(row.state);
            let kind = row.kind.clone().unwrap_or_else(|| "?".to_owned());
            let visits = if row.visits > 1 {
                format!(" ×{}", row.visits)
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", state_glyph(row.state)), style),
                Span::styled(format!("{:<12}", row.id), style),
                Span::styled(
                    format!("{kind}{visits} — {}", state_word(row.state)),
                    Style::new().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();
    List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" pipeline "))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
}

fn node_style(state: wire::NodeState) -> Style {
    match state {
        wire::NodeState::Running => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        wire::NodeState::Succeeded => Style::new().fg(Color::Green),
        wire::NodeState::Failed => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        wire::NodeState::Skipped => Style::new().fg(Color::DarkGray),
        wire::NodeState::AwaitingApproval => {
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        }
        wire::NodeState::Cancelled => Style::new().fg(Color::Magenta),
        _ => Style::new().fg(Color::Gray),
    }
}
