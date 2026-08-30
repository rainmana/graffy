//! Graph review — tiers two and three of the founding design.
//!
//! * **Guided** (`collaborative = false`): inspect the generated-but-
//!   unregistered graph; rename it; accept or reject. Nothing persists until
//!   a human accepts; reject leaves no trace.
//! * **Collaborative** (`collaborative = true`): everything guided offers,
//!   plus live co-design of the selected node — edit its description
//!   inline, cycle its routing tier, and open its system knowledge in
//!   `$EDITOR` (suspending the TUI, the terminal-native way). Structural
//!   edits (adding/removing nodes and edges) are the next increment.
//!
//! Accept runs the cycle-guard compiler first: an edit that would produce an
//! unlawful graph cannot be registered — the error shows in the footer.

use anyhow::Result;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::time::Duration;

use graffy_core::graph::CompiledGraph;
use graffy_core::spec::GraphSpec;

/// The human's verdict. Edits (collaborative) are applied to the spec in
/// place before this is returned; Reject discards everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Accept,
    Reject,
}

/// Cycle a node's routing tier: none → fast → balanced → frontier → none.
/// Unknown custom tiers clear first, then cycle the standard ladder.
pub fn next_tier(current: Option<&str>) -> Option<String> {
    match current {
        None => Some("fast".to_owned()),
        Some("fast") => Some("balanced".to_owned()),
        Some("balanced") => Some("frontier".to_owned()),
        Some("frontier") => None,
        Some(_) => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTarget {
    GraphName,
    NodeDescription,
}

struct ReviewState {
    list: ListState,
    scroll: u16,
    editing: Option<EditTarget>,
    buffer: String,
    error: Option<String>,
    dirty: bool,
}

/// Open the review TUI. Guided allows rename only; collaborative allows
/// node-level co-design. Nothing is registered by this function.
pub fn review_spec(spec: &mut GraphSpec, collaborative: bool) -> Result<ReviewDecision> {
    let mut state = ReviewState {
        list: ListState::default(),
        scroll: 0,
        editing: None,
        buffer: String::new(),
        error: None,
        dirty: false,
    };
    state.list.select(Some(0));

    let mut terminal = ratatui::init();
    let result = review_loop(&mut terminal, spec, collaborative, &mut state);
    ratatui::restore();
    result
}

fn review_loop(
    terminal: &mut DefaultTerminal,
    spec: &mut GraphSpec,
    collaborative: bool,
    state: &mut ReviewState,
) -> Result<ReviewDecision> {
    loop {
        terminal.draw(|f| draw(f, spec, collaborative, state))?;
        if event::poll(Duration::from_millis(100))?
            && let TermEvent::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if let Some(target) = state.editing {
                match key.code {
                    KeyCode::Enter => {
                        let value = state.buffer.trim().to_owned();
                        if !value.is_empty() {
                            match target {
                                EditTarget::GraphName => spec.graph.name = value,
                                EditTarget::NodeDescription => {
                                    if let Some(node) = selected_node_mut(spec, &state.list) {
                                        node.description = value;
                                    }
                                }
                            }
                            state.dirty = true;
                        }
                        state.editing = None;
                    }
                    KeyCode::Esc => state.editing = None,
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
                    // The compiler is the gate: unlawful edits cannot land.
                    match CompiledGraph::compile(spec) {
                        Ok(_) => return Ok(ReviewDecision::Accept),
                        Err(err) => state.error = Some(err.to_string()),
                    }
                }
                KeyCode::Char('r') | KeyCode::Char('q') | KeyCode::Esc => {
                    return Ok(ReviewDecision::Reject);
                }
                KeyCode::Char('n') => {
                    state.buffer = spec.graph.name.clone();
                    state.editing = Some(EditTarget::GraphName);
                }
                KeyCode::Char('d') if collaborative => {
                    if let Some(node) = selected_node_mut(spec, &state.list) {
                        state.buffer = node.description.clone();
                        state.editing = Some(EditTarget::NodeDescription);
                    }
                }
                KeyCode::Char('t') if collaborative => {
                    if let Some(node) = selected_node_mut(spec, &state.list) {
                        node.model_tier = next_tier(node.model_tier.as_deref());
                        state.dirty = true;
                    }
                }
                KeyCode::Char('s') if collaborative => {
                    let is_model = selected_node_mut(spec, &state.list)
                        .map(|n| n.kind == "model")
                        .unwrap_or(false);
                    if is_model {
                        let initial = selected_node_mut(spec, &state.list)
                            .and_then(|n| {
                                n.params
                                    .get("system")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_owned)
                            })
                            .unwrap_or_default();
                        if let Some(edited) = edit_in_editor(terminal, &initial)?
                            && let Some(node) = selected_node_mut(spec, &state.list)
                        {
                            node.params.insert(
                                "system".to_owned(),
                                toml::Value::String(edited.trim_end().to_owned()),
                            );
                            state.dirty = true;
                        }
                    }
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

/// Suspend the TUI, open `$VISUAL`/`$EDITOR` (default `vi`) on the text,
/// resume, and return the edited content (None on editor failure/abort).
fn edit_in_editor(terminal: &mut DefaultTerminal, initial: &str) -> Result<Option<String>> {
    ratatui::restore();
    let path = std::env::temp_dir().join(format!(
        "graffy-edit-{}-{}.md",
        std::process::id(),
        graffy_core::id::RunId::generate()
    ));
    std::fs::write(&path, initial)?;
    let editor_cmd = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_owned());
    let mut parts = editor_cmd.split_whitespace();
    let program = parts.next().unwrap_or("vi").to_owned();
    let args: Vec<String> = parts.map(str::to_owned).collect();
    let status = std::process::Command::new(&program)
        .args(&args)
        .arg(&path)
        .status();
    let edited = match status {
        Ok(s) if s.success() => Some(std::fs::read_to_string(&path)?),
        _ => None,
    };
    std::fs::remove_file(&path).ok();
    *terminal = ratatui::init();
    Ok(edited)
}

fn selected_node_mut<'a>(
    spec: &'a mut GraphSpec,
    list: &ListState,
) -> Option<&'a mut graffy_core::spec::NodeSpec> {
    list.selected().and_then(|i| spec.nodes.get_mut(i))
}

fn select_delta(list: &mut ListState, len: usize, delta: i64) {
    if len == 0 {
        return;
    }
    let current = list.selected().unwrap_or(0) as i64;
    let next = (current + delta).rem_euclid(len as i64) as usize;
    list.select(Some(next));
}

fn draw(f: &mut Frame, spec: &GraphSpec, collaborative: bool, state: &mut ReviewState) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .areas(f.area());

    let mode_label = if collaborative {
        " collaborative review "
    } else {
        " guided review "
    };
    let title = Line::from(vec![
        Span::styled(
            mode_label,
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("▸ {} ", spec.graph.name)),
        Span::styled(
            format!("({} · v{})", spec.graph.id, spec.graph.version),
            Style::new().fg(Color::DarkGray),
        ),
        if state.dirty {
            Span::styled(" · edited", Style::new().fg(Color::Cyan))
        } else {
            Span::raw("")
        },
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

    let footer_line = if let Some(target) = state.editing {
        let label = match target {
            EditTarget::GraphName => "rename graph",
            EditTarget::NodeDescription => "edit description",
        };
        Line::from(vec![
            Span::styled(format!(" {label}: "), Style::new().fg(Color::Yellow)),
            Span::raw(format!("{}▏", state.buffer)),
            Span::styled(
                "   Enter save · Esc cancel",
                Style::new().fg(Color::DarkGray),
            ),
        ])
    } else if let Some(err) = &state.error {
        Line::from(Span::styled(
            format!(" cannot register: {err}"),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))
    } else if collaborative {
        Line::from(Span::styled(
            " [a] accept · [r]/q reject · [n] rename · [d] description · [t] tier · [s] system in $EDITOR (model nodes) · ↑↓ · j/k",
            Style::new().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(
            " [a]/Enter accept & register · [n] rename · [r]/q reject · ↑↓ node · j/k scroll",
            Style::new().fg(Color::DarkGray),
        ))
    };
    let hint = Line::from(Span::styled(
        if collaborative {
            " collaborative mode: co-design the graph, then you are the gate — the compiler blocks unlawful edits"
        } else {
            " guided mode: you are the gate — accept registers, reject leaves no trace"
        },
        Style::new()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    ));
    f.render_widget(Paragraph::new(vec![footer_line, hint]), footer);
}

#[cfg(test)]
mod tests {
    use super::next_tier;

    #[test]
    fn tier_cycles_standard_ladder_and_clears_custom() {
        assert_eq!(next_tier(None).as_deref(), Some("fast"));
        assert_eq!(next_tier(Some("fast")).as_deref(), Some("balanced"));
        assert_eq!(next_tier(Some("balanced")).as_deref(), Some("frontier"));
        assert_eq!(next_tier(Some("frontier")), None);
        assert_eq!(next_tier(Some("my-custom-tier")), None);
    }
}
