//! Guided graphification (founding tier two): render a *generated, not yet
//! registered* graph spec for human review. Accept registers it, rename
//! adjusts the display name, reject discards it — nothing persists until a
//! human says so. Collaborative (node-by-node co-design) is tier three and
//! builds on this surface.

use anyhow::Result;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::time::Duration;

use graffy_core::spec::GraphSpec;

/// The human's verdict on a previewed spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewDecision {
    Accept,
    /// Accept with a new display name (the id stays stable).
    AcceptRenamed(String),
    Reject,
}

struct PreviewState {
    list: ListState,
    scroll: u16,
    renaming: bool,
    buffer: String,
    renamed: Option<String>,
}

/// Open the review TUI for a generated spec. Returns the decision; nothing
/// is registered by this function under any outcome.
pub fn preview_spec(spec: &GraphSpec) -> Result<PreviewDecision> {
    let mut state = PreviewState {
        list: ListState::default(),
        scroll: 0,
        renaming: false,
        buffer: String::new(),
        renamed: None,
    };
    state.list.select(Some(0));

    let mut terminal = ratatui::init();
    let result = preview_loop(&mut terminal, spec, &mut state);
    ratatui::restore();
    result
}

fn preview_loop(
    terminal: &mut DefaultTerminal,
    spec: &GraphSpec,
    state: &mut PreviewState,
) -> Result<PreviewDecision> {
    loop {
        terminal.draw(|f| draw(f, spec, state))?;
        if event::poll(Duration::from_millis(100))?
            && let TermEvent::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if state.renaming {
                match key.code {
                    KeyCode::Enter => {
                        let trimmed = state.buffer.trim();
                        if !trimmed.is_empty() {
                            state.renamed = Some(trimmed.to_owned());
                        }
                        state.renaming = false;
                    }
                    KeyCode::Esc => state.renaming = false,
                    KeyCode::Backspace => {
                        state.buffer.pop();
                    }
                    KeyCode::Char(c) => state.buffer.push(c),
                    _ => {}
                }
                continue;
            }
            match key.code {
                KeyCode::Char('a') | KeyCode::Enter => {
                    return Ok(match state.renamed.take() {
                        Some(name) => PreviewDecision::AcceptRenamed(name),
                        None => PreviewDecision::Accept,
                    });
                }
                KeyCode::Char('r') | KeyCode::Char('q') | KeyCode::Esc => {
                    return Ok(PreviewDecision::Reject);
                }
                KeyCode::Char('e') => {
                    state.buffer = state
                        .renamed
                        .clone()
                        .unwrap_or_else(|| spec.graph.name.clone());
                    state.renaming = true;
                }
                KeyCode::Up => select_delta(&mut state.list, spec.nodes.len(), -1),
                KeyCode::Down => select_delta(&mut state.list, spec.nodes.len(), 1),
                KeyCode::Char('j') | KeyCode::PageDown => {
                    state.scroll = state.scroll.saturating_add(3);
                }
                KeyCode::Char('k') | KeyCode::PageUp => {
                    state.scroll = state.scroll.saturating_sub(3);
                }
                _ => {}
            }
        }
    }
}

fn select_delta(list: &mut ListState, len: usize, delta: i64) {
    if len == 0 {
        return;
    }
    let current = list.selected().unwrap_or(0) as i64;
    let next = (current + delta).rem_euclid(len as i64) as usize;
    list.select(Some(next));
}

fn draw(f: &mut Frame, spec: &GraphSpec, state: &mut PreviewState) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .areas(f.area());

    let shown_name = state.renamed.as_deref().unwrap_or(&spec.graph.name);
    let title = Line::from(vec![
        Span::styled(
            " guided review ",
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("▸ {shown_name} ")),
        Span::styled(
            format!("({} · v{})", spec.graph.id, spec.graph.version),
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    let status = Line::from(Span::styled(
        " nothing is registered yet — this graph exists only on this screen",
        Style::new()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    ));
    f.render_widget(Paragraph::new(vec![title, status]), header);

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(34), Constraint::Percentage(66)]).areas(body);

    let items: Vec<ListItem> = spec
        .nodes
        .iter()
        .map(|n| {
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<12}", n.id), Style::new().fg(Color::Cyan)),
                Span::styled(
                    format!(
                        "{}{}",
                        n.kind,
                        n.model_tier
                            .as_deref()
                            .map(|t| format!(" · {t}"))
                            .unwrap_or_default()
                    ),
                    Style::new().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" nodes "))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut state.list,
    );

    let mut lines: Vec<Line> = Vec::new();
    if let Some(node) = state.list.selected().and_then(|i| spec.nodes.get(i)) {
        lines.push(Line::from(Span::styled(
            format!("{} ({})", node.id, node.kind),
            Style::new().add_modifier(Modifier::BOLD),
        )));
        if !node.description.is_empty() {
            lines.push(Line::from(node.description.clone()));
        }
        for (key, value) in &node.params {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("param: {key}"),
                Style::new().fg(Color::Yellow),
            )));
            let rendered = match value.as_str() {
                Some(s) => s.to_owned(),
                None => value.to_string(),
            };
            for text_line in rendered.lines().take(18) {
                lines.push(Line::from(format!("  {text_line}")));
            }
            if rendered.lines().count() > 18 {
                lines.push(Line::from("  … (full text lands in the registered TOML)"));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "edges touching this node:",
            Style::new().fg(Color::Yellow),
        )));
        for edge in &spec.edges {
            if edge.from == node.id || edge.to == node.id {
                let guard = edge
                    .when
                    .as_deref()
                    .map(|g| format!("   when {g}"))
                    .unwrap_or_default();
                lines.push(Line::from(format!(
                    "  {} → {}{}",
                    edge.from, edge.to, guard
                )));
            }
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" detail "))
            .wrap(Wrap { trim: false })
            .scroll((state.scroll, 0)),
        right,
    );

    let footer_line = if state.renaming {
        Line::from(vec![
            Span::styled(" rename: ", Style::new().fg(Color::Yellow)),
            Span::raw(format!("{}▏", state.buffer)),
            Span::styled(
                "   Enter save · Esc cancel",
                Style::new().fg(Color::DarkGray),
            ),
        ])
    } else {
        Line::from(Span::styled(
            " [a]/Enter accept & register · [e] rename · [r]/q reject · ↑↓ node · j/k scroll",
            Style::new().fg(Color::DarkGray),
        ))
    };
    let hint = Line::from(Span::styled(
        " guided mode: you are the gate — accept registers, reject leaves no trace",
        Style::new()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    ));
    f.render_widget(Paragraph::new(vec![footer_line, hint]), footer);
}
