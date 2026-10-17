//! Terminal rendering + event loops (ADR-0002, Phase 1 M3 + approvals).
//!
//! Entry points:
//! * [`run_live`] — execute a graph while rendering the journal tap in real
//!   time; PauseForDisambiguation checkpoints freeze the view with an
//!   approve / reject prompt; stays open for inspection afterwards.
//! * [`run_replay`] — fold a finished journal and open the step inspector.
//! * [`run_home`] — pick a journal from the runs directory and inspect it.
//!
//! Keys: `q`/`Esc` quit · `Tab` switch Run/Inspect · `↑`/`↓` select node ·
//! `j`/`k` scroll detail. During an approval: `a` approve · `r` reject ·
//! arrows scroll the exact payload. Quitting cancels an unfinished run;
//! EOF/closed UI is not recorded as a human decision.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use graffy_core::error::ExecError;
use graffy_core::exec::{
    ApprovalHandler, ApprovalOutcome, Executor, ModelInvoker, RunInput, RunOutcome, ToolInvoker,
};
use graffy_core::journal::{JournalReader, wire};
use graffy_core::spec::GraphSpec;

use crate::state::{AppState, state_glyph, state_word, status_word};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Run,
    Inspect,
}

/// A pending human decision surfaced from the executor.
struct ApprovalPrompt {
    node_id: String,
    question: String,
    scroll: u16,
    reply: tokio::sync::oneshot::Sender<ApprovalOutcome>,
}

struct UiState {
    view: View,
    node_list: ListState,
    detail_scroll: u16,
    approval: Option<ApprovalPrompt>,
}

impl UiState {
    fn new(view: View) -> Self {
        Self {
            view,
            node_list: ListState::default(),
            detail_scroll: 0,
            approval: None,
        }
    }
}

/// Wire between the executor task and the UI loop.
pub struct ApprovalRequest {
    pub node_id: String,
    pub question: String,
    pub reply: tokio::sync::oneshot::Sender<ApprovalOutcome>,
}

/// [`ApprovalHandler`] that forwards checkpoints to the live TUI and blocks
/// the executor task until a human decides. If the UI is gone, the safe
/// default is rejection — silence never releases anything.
pub struct TuiApprovalHandler {
    tx: tokio::sync::mpsc::UnboundedSender<ApprovalRequest>,
}

#[async_trait::async_trait]
impl ApprovalHandler for TuiApprovalHandler {
    fn describe(&self) -> &'static str {
        "human-tui"
    }

    fn is_human(&self) -> bool {
        true
    }

    async fn resolve(&self, node_id: &str, question: &str) -> ApprovalOutcome {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let request = ApprovalRequest {
            node_id: node_id.to_owned(),
            question: question.to_owned(),
            reply: reply_tx,
        };
        if self.tx.send(request).is_err() {
            return ApprovalOutcome::Abandoned;
        }
        reply_rx.await.unwrap_or(ApprovalOutcome::Abandoned)
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
    tool_invoker: Option<Arc<dyn ToolInvoker>>,
    remaining_budget: graffy_core::control::RemainingBudget,
) -> Result<Option<RunOutcome>> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (approval_tx, mut approval_rx) = tokio::sync::mpsc::unbounded_channel();
    let cancellation = graffy_core::control::Cancellation::default();
    let executor = Executor {
        cancellation: cancellation.clone(),
        event_tap: Some(tx),
        tool_invoker,
        remaining_budget,
        ..Default::default()
    };

    let spec_for_task = spec.clone();
    let path_for_task = journal_path.clone();
    let task: tokio::task::JoinHandle<Result<RunOutcome, ExecError>> = tokio::spawn(async move {
        let approvals = TuiApprovalHandler { tx: approval_tx };
        executor
            .run(
                &spec_for_task,
                &spec_toml,
                RunInput {
                    prompt,
                    session_id: None,
                    feedback: Vec::new(),
                    ..Default::default()
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
    let loop_result = live_loop(
        &mut terminal,
        &mut rx,
        &mut approval_rx,
        &task,
        &mut app,
        &mut ui,
    );
    ratatui::restore();
    approval_rx.close();
    ui.approval.take();
    if !task.is_finished() {
        cancellation.cancel();
        eprintln!("Run cancellation requested; already-started tool effects may have occurred.");
    }
    let task_result = task.await;
    loop_result?;
    match task_result {
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

/// Journal picker over the runs directory, opening the inspector on selection.
pub fn run_home(dir: &Path) -> Result<()> {
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
        println!("no journals in {} yet.", dir.display());
        println!("try: graffy run graffy.builtin.conversation --prompt \"hello\" --offline --tui");
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
                            .title(" run journals "),
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
    approvals: &mut tokio::sync::mpsc::UnboundedReceiver<ApprovalRequest>,
    task: &tokio::task::JoinHandle<Result<RunOutcome, ExecError>>,
    app: &mut AppState,
    ui: &mut UiState,
) -> Result<()> {
    loop {
        while let Ok(frame) = rx.try_recv() {
            app.apply(&frame);
        }
        while let Ok(request) = approvals.try_recv() {
            ui.approval = Some(ApprovalPrompt {
                node_id: request.node_id,
                question: request.question,
                scroll: 0,
                reply: request.reply,
            });
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

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Returns true when the user asked to quit.
fn handle_key(code: KeyCode, app: &AppState, ui: &mut UiState) -> bool {
    if ui.approval.is_some() {
        return handle_approval_key(code, ui);
    }
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

fn handle_approval_key(code: KeyCode, ui: &mut UiState) -> bool {
    match code {
        KeyCode::Char('a') => respond_approval(ui, ApprovalOutcome::ApprovedInput("a".into())),
        KeyCode::Char('r') => respond_approval(ui, ApprovalOutcome::RejectedInput("r".into())),
        KeyCode::Char('q') | KeyCode::Esc => {
            respond_approval(ui, ApprovalOutcome::Abandoned);
            return true;
        }
        KeyCode::Down | KeyCode::PageDown => {
            if let Some(prompt) = &mut ui.approval {
                prompt.scroll = prompt.scroll.saturating_add(1);
            }
        }
        KeyCode::Up | KeyCode::PageUp => {
            if let Some(prompt) = &mut ui.approval {
                prompt.scroll = prompt.scroll.saturating_sub(1);
            }
        }
        _ => {}
    }
    false
}

fn respond_approval(ui: &mut UiState, outcome: ApprovalOutcome) {
    if let Some(prompt) = ui.approval.take() {
        let _ = prompt.reply.send(outcome);
    }
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

    if ui.approval.is_some() {
        let [header, body, modal, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(4),
            Constraint::Length(2),
        ])
        .areas(f.area());
        draw_header(f, app, header);
        draw_body(f, app, ui, body);
        draw_modal(f, ui, modal);
        draw_footer(f, app, footer, finished);
    } else {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .areas(f.area());
        draw_header(f, app, header);
        draw_body(f, app, ui, body);
        draw_footer(f, app, footer, finished);
    }
}

fn draw_header(f: &mut Frame, app: &AppState, area: Rect) {
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
    f.render_widget(Paragraph::new(vec![title, counters]), area);
}

fn draw_body(f: &mut Frame, app: &AppState, ui: &mut UiState, area: Rect) {
    match ui.view {
        View::Run => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
                    .areas(area);
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
                    .areas(area);
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
}

fn draw_modal(f: &mut Frame, ui: &UiState, area: Rect) {
    let Some(prompt) = &ui.approval else { return };
    f.render_widget(
        Paragraph::new(prompt.question.as_str())
            .wrap(Wrap { trim: false })
            .scroll((prompt.scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" approval — {} ", prompt.node_id))
                    .title_bottom("[a] approve · [r] reject · ↑/↓ scroll · q cancels")
                    .border_style(Style::new().fg(Color::Yellow)),
            ),
        area,
    );
}

fn draw_footer(f: &mut Frame, app: &AppState, area: Rect, finished: bool) {
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
    f.render_widget(Paragraph::new(vec![novice, keys]), area);
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

#[cfg(test)]
mod release_tests {
    use super::*;
    #[tokio::test]
    async fn closing_the_ui_abandons_current_and_future_approval_requests() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handler = Arc::new(TuiApprovalHandler { tx });
        let active = handler.clone();
        let task = tokio::spawn(async move { active.resolve("gate", "payload").await });
        let request = rx.recv().await.unwrap();
        rx.close();
        drop(request);
        assert_eq!(task.await.unwrap(), ApprovalOutcome::Abandoned);
        assert_eq!(
            handler.resolve("future", "payload").await,
            ApprovalOutcome::Abandoned
        );
    }
    #[tokio::test]
    async fn edit_key_cannot_approve_and_quit_abandons() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut ui = UiState::new(View::Run);
        ui.approval = Some(ApprovalPrompt {
            node_id: "gate".into(),
            question: "payload".into(),
            scroll: 0,
            reply: tx,
        });
        assert!(!handle_approval_key(KeyCode::Char('e'), &mut ui));
        assert!(ui.approval.is_some());
        assert!(handle_approval_key(KeyCode::Esc, &mut ui));
        assert_eq!(rx.await.unwrap(), ApprovalOutcome::Abandoned);
    }
    #[test]
    fn approval_payload_wraps_and_scrolls_on_small_terminals() {
        let (tx, _) = tokio::sync::oneshot::channel();
        let mut ui = UiState::new(View::Run);
        ui.approval = Some(ApprovalPrompt {
            node_id: "gate".into(),
            question: "Exact payload:\nFIRST\nline 2\nline 3\nline 4\nline 5\nLAST".into(),
            scroll: 0,
            reply: tx,
        });
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 7)).unwrap();
        terminal.draw(|f| draw_modal(f, &ui, f.area())).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("FIRST"));
        assert!(!text.contains("LAST"));
        for _ in 0..3 {
            handle_approval_key(KeyCode::Down, &mut ui);
        }
        terminal.draw(|f| draw_modal(f, &ui, f.area())).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("LAST"));
        for (w, h) in [(1, 1), (10, 3)] {
            let mut tiny =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            tiny.draw(|f| draw_modal(f, &ui, f.area())).unwrap();
        }
    }
}
