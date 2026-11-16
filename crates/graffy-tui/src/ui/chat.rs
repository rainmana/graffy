//! Conversation view (ADR-0010 step 3): composer, scrollable transcript, one
//! active turn at a time, inline approvals, and the existing step inspector
//! as optional run details.
//!
//! The view owns no execution. The caller's `start` closure runs each turn
//! (through the shared turn runner) and hands back a [`ChatReply`]; this
//! module only renders what was recorded and released. An answer's output
//! receipt is recorded after the frame that shows it has been drawn.

use std::pin::Pin;

use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, KeyEvent,
    KeyModifiers, MouseEventKind,
};
use ratatui::crossterm::execute;

use super::*;

/// One submitted message.
pub struct ChatTurn {
    pub prompt: String,
    /// Explicit human declaration (`/correct`), never inferred from wording.
    pub correction: bool,
}

/// What a finished turn hands back to the view.
#[derive(Default)]
pub struct ChatReply {
    /// (honest label, content) of the released answer. `None`: nothing was
    /// released, and nothing is shown as an answer.
    pub answer: Option<(String, String)>,
    /// Plain-language notes: failed attempts, retries, warnings.
    pub notices: Vec<String>,
    /// The session after this turn, for the header.
    pub session: Option<String>,
    /// Awaited once the answer has actually been drawn; records the receipt.
    pub confirm_shown: Option<Pin<Box<dyn Future<Output = Result<()>> + Send>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Human,
    Ai,
    Note,
}

struct Entry {
    who: Who,
    label: String,
    text: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    None,
    Submit {
        prompt: String,
        correction: bool,
    },
    NewSession,
    /// Stop the turn in flight.
    Cancel,
    Quit,
    /// Open the step inspector on the latest run.
    Details,
}

/// Terminal-free conversation state — everything the keys can change.
#[derive(Default)]
struct ChatState {
    entries: Vec<Entry>,
    input: String,
    /// Lines scrolled up from the newest line (0 = following the bottom).
    scroll_back: u16,
    busy: bool,
    hint: Option<String>,
}

/// Recorded text is data, not terminal instructions: drop control characters
/// (escape sequences in model output must not reach the terminal).
fn clean(text: &str) -> String {
    text.replace('\t', "    ")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect()
}

impl ChatState {
    fn push(&mut self, who: Who, label: &str, text: &str) {
        self.entries.push(Entry {
            who,
            label: label.to_owned(),
            text: clean(text),
        });
        self.scroll_back = 0;
    }

    /// Scroll the transcript; positive is toward older messages.
    fn scroll(&mut self, lines: i16) {
        self.scroll_back = self.scroll_back.saturating_add_signed(lines);
    }

    fn on_paste(&mut self, text: &str) {
        self.input
            .push_str(&clean(&text.replace("\r\n", "\n").replace('\r', "\n")));
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => {
                return if self.busy {
                    Action::Cancel
                } else {
                    Action::Quit
                };
            }
            KeyCode::Enter => return self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            // Scrolling needs no PageUp/PageDown key (laptop keyboards): arrows
            // move a line, Ctrl-U / Ctrl-D half a page as in vim. Plain letters
            // stay text, so j/k cannot scroll while the composer has focus.
            KeyCode::Up => self.scroll(1),
            KeyCode::Down => self.scroll(-1),
            KeyCode::Char('u') if ctrl => self.scroll(10),
            KeyCode::Char('d') if ctrl => self.scroll(-10),
            KeyCode::PageUp => self.scroll(10),
            KeyCode::PageDown => self.scroll(-10),
            KeyCode::Tab if !self.busy => return Action::Details,
            // ponytail: append-only editing (no cursor movement inside the
            // draft). Add a cursor index if editing mid-line is missed.
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.input.push(c);
                self.hint = None;
            }
            _ => {}
        }
        Action::None
    }

    fn submit(&mut self) -> Action {
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return Action::None;
        }
        if text == "/quit" {
            return Action::Quit;
        }
        if self.busy {
            // The draft is kept, never queued or sent behind the user's back.
            self.hint = Some(
                "still working — your draft is kept. Press Enter again when this turn \
                 finishes, or Ctrl-C to stop it."
                    .into(),
            );
            return Action::None;
        }
        self.input.clear();
        self.hint = None;
        if text == "/new" {
            self.push(
                Who::Note,
                "note",
                "new conversation — earlier messages above are not carried into it",
            );
            return Action::NewSession;
        }
        let (prompt, correction) = match text.strip_prefix("/correct ") {
            Some(rest) => (rest.trim().to_owned(), true),
            None => (text, false),
        };
        self.push(
            Who::Human,
            if correction {
                "you · correction"
            } else {
                "you"
            },
            &prompt,
        );
        Action::Submit { prompt, correction }
    }

    /// The transcript as pre-wrapped lines. Every speaker has a glyph AND a
    /// word — never color alone.
    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for entry in &self.entries {
            let (glyph, color) = match entry.who {
                Who::Human => ("▸", Color::Cyan),
                Who::Ai => ("◂", Color::Green),
                Who::Note => ("!", Color::Yellow),
            };
            out.push(Line::from(Span::styled(
                format!("{glyph} {}", entry.label),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            )));
            for line in wrap(&entry.text, width.saturating_sub(2) as usize) {
                out.push(Line::from(format!("  {line}")));
            }
            out.push(Line::default());
        }
        out
    }
}

/// Greedy word wrap by display width; over-long words are split. A line's
/// leading indentation is kept, and repeated on its continuation lines, so
/// code and YAML in a recorded answer keep their structure.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let full_width = width.max(1);
    let cells = |s: &str| Span::raw(s.to_owned()).width();
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let body = paragraph.trim_start_matches(' ');
        // Never let indentation take more than half the line.
        let indent = " ".repeat((paragraph.len() - body.len()).min(full_width / 2));
        let width = full_width - indent.len();
        let paragraph = body;
        let first = out.len();
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let mut word = word.to_owned();
            if !line.is_empty() && cells(&line) + 1 + cells(&word) > width {
                out.push(std::mem::take(&mut line));
            }
            while cells(&word) > width {
                let mut head = String::new();
                while let Some(c) = word.chars().next() {
                    if !head.is_empty() && cells(&head) + cells(&c.to_string()) > width {
                        break;
                    }
                    head.push(c);
                    word.drain(..c.len_utf8());
                }
                out.push(head);
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        out.push(line);
        for line in &mut out[first..] {
            if !line.is_empty() {
                line.insert_str(0, &indent);
            }
        }
    }
    out
}

/// Approval keys inside chat. The composer invites typing, so a plain letter
/// must never decide a gate: only deliberate Ctrl chords do. Returns false
/// for keys that were ignored.
fn approval_key(key: KeyEvent, ui: &mut UiState) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('a') if ctrl => {
            respond_approval(ui, ApprovalOutcome::ApprovedInput("ctrl-a".into()))
        }
        KeyCode::Char('r') if ctrl => {
            respond_approval(ui, ApprovalOutcome::RejectedInput("ctrl-r".into()))
        }
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
            handle_approval_key(key.code, ui);
        }
        _ => return false,
    }
    true
}

struct ActiveTurn {
    task: tokio::task::JoinHandle<Result<ChatReply>>,
    cancellation: Cancellation,
    tap: tokio::sync::mpsc::UnboundedReceiver<wire::RunEvent>,
}

/// Run the conversation view until the user leaves.
///
/// * `history` — the recorded visible transcript of a resumed session.
/// * `start` — executes one turn; must go through the shared turn runner.
/// * `new_session` — the user asked for a fresh conversation (`/new`).
pub async fn run_chat<S, Fut, N>(
    spec: &GraphSpec,
    history: Vec<(String, String)>,
    mut session: Option<String>,
    mut start: S,
    mut new_session: N,
) -> Result<()>
where
    S: FnMut(ChatTurn, EventTap, Arc<dyn ApprovalHandler>, Cancellation) -> Fut,
    Fut: Future<Output = Result<ChatReply>> + Send + 'static,
    N: FnMut(),
{
    let (approval_tx, mut approval_rx) = tokio::sync::mpsc::unbounded_channel();
    let approvals: Arc<dyn ApprovalHandler> = Arc::new(TuiApprovalHandler { tx: approval_tx });
    let mut chat = ChatState::default();
    for (actor, content) in &history {
        match actor.as_str() {
            "human" => chat.push(Who::Human, "you", content),
            _ => chat.push(Who::Ai, "graffy · earlier response", content),
        }
    }
    let mut app = AppState::default();
    let mut ui = UiState::new(View::Inspect);
    let mut active: Option<ActiveTurn> = None;
    let mut confirm: Option<Pin<Box<dyn Future<Output = Result<()>> + Send>>> = None;

    let mut terminal = ratatui::init();
    // Without bracketed paste, a pasted newline would submit a half message.
    // Mouse capture gives wheel scrolling. Cost: selecting text then needs the
    // terminal's modifier (Option on macOS terminals, Shift on most others).
    let _ = execute!(std::io::stdout(), EnableBracketedPaste, EnableMouseCapture);
    let result: Result<()> = async {
        loop {
            if let Some(turn) = &mut active {
                while let Ok(frame) = turn.tap.try_recv() {
                    if matches!(frame.event, Some(wire::run_event::Event::RunStarted(_))) {
                        app = AppState::default();
                        app.seed_from_spec(spec);
                    }
                    app.apply(&frame);
                }
                while let Ok(request) = approval_rx.try_recv() {
                    ui.approval = Some(ApprovalPrompt {
                        node_id: request.node_id,
                        question: request.question,
                        scroll: 0,
                        reply: request.reply,
                    });
                }
            }
            if active.as_ref().is_some_and(|t| t.task.is_finished()) {
                let turn = active.take().expect("checked above");
                ui.approval = None;
                chat.busy = false;
                match turn.task.await {
                    Ok(Ok(reply)) => {
                        for notice in &reply.notices {
                            chat.push(Who::Note, "note", notice);
                        }
                        match reply.answer {
                            Some((label, content)) => {
                                chat.push(Who::Ai, &format!("graffy · {label}"), &content);
                                confirm = reply.confirm_shown;
                            }
                            None => {
                                chat.push(Who::Note, "note", "no answer was released for this turn")
                            }
                        }
                        if reply.session.is_some() {
                            session = reply.session;
                        }
                    }
                    Ok(Err(err)) => chat.push(
                        Who::Note,
                        "note",
                        &format!("turn failed: {err} — no answer was released"),
                    ),
                    Err(join) => chat.push(Who::Note, "note", &format!("turn crashed: {join}")),
                }
            }

            terminal.draw(|f| draw_chat(f, spec, session.as_deref(), &chat, &app, &ui))?;
            // The answer is on screen now — only now may it count as delivered.
            if let Some(shown) = confirm.take()
                && let Err(err) = shown.await
            {
                chat.push(
                    Who::Note,
                    "note",
                    &format!("warning: the delivery receipt could not be recorded: {err}"),
                );
            }

            if !event::poll(Duration::from_millis(50))? {
                continue;
            }
            let action = match event::read()? {
                TermEvent::Mouse(mouse) => {
                    let lines = match mouse.kind {
                        MouseEventKind::ScrollUp => 3,
                        MouseEventKind::ScrollDown => -3,
                        _ => 0,
                    };
                    match &mut ui.approval {
                        // With a payload on screen the wheel scrolls the payload.
                        Some(prompt) => prompt.scroll = prompt.scroll.saturating_add_signed(-lines),
                        None => chat.scroll(lines),
                    }
                    Action::None
                }
                TermEvent::Paste(text) if ui.approval.is_none() => {
                    chat.on_paste(&text);
                    Action::None
                }
                TermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    let ctrl_c = key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL);
                    if ui.approval.is_some() && !ctrl_c {
                        if !approval_key(key, &mut ui) {
                            chat.hint = Some(
                                "an approval is waiting — Ctrl-A approves, Ctrl-R rejects; \
                                 typing is paused so a keystroke cannot decide it"
                                    .into(),
                            );
                        }
                        Action::None
                    } else {
                        chat.on_key(key)
                    }
                }
                _ => Action::None,
            };
            match action {
                Action::None => {}
                Action::Quit => return Ok(()),
                Action::Cancel => {
                    respond_approval(&mut ui, ApprovalOutcome::Abandoned);
                    if let Some(turn) = &active {
                        turn.cancellation.cancel();
                        chat.push(
                            Who::Note,
                            "note",
                            "stopping this turn — tool effects already performed are not undone",
                        );
                    }
                }
                Action::NewSession => {
                    new_session();
                    session = None;
                }
                Action::Details => {
                    if app.run_id.is_empty() {
                        chat.hint = Some("no run to inspect yet — send a message first".into());
                    } else {
                        // Read-only: opening details never executes anything.
                        static_loop(&mut terminal, &app, &mut ui)?;
                    }
                }
                Action::Submit { prompt, correction } => {
                    let (tap_tx, tap) = tokio::sync::mpsc::unbounded_channel();
                    let cancellation = Cancellation::default();
                    let task = tokio::spawn(start(
                        ChatTurn { prompt, correction },
                        tap_tx,
                        approvals.clone(),
                        cancellation.clone(),
                    ));
                    active = Some(ActiveTurn {
                        task,
                        cancellation,
                        tap,
                    });
                    chat.busy = true;
                }
            }
        }
    }
    .await;
    let _ = execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    );
    ratatui::restore();

    // Leaving with a turn in flight cancels it; EOF/closed UI is never a decision.
    approval_rx.close();
    ui.approval.take();
    if let Some(turn) = active {
        turn.cancellation.cancel();
        eprintln!("Turn cancellation requested; already-started tool effects may have occurred.");
        let _ = turn.task.await;
    }
    result
}

fn draw_chat(
    f: &mut Frame,
    spec: &GraphSpec,
    session: Option<&str>,
    chat: &ChatState,
    app: &AppState,
    ui: &UiState,
) {
    let draft: Vec<String> = wrap(
        &format!("> {}", chat.input),
        f.area().width.saturating_sub(2) as usize,
    );
    let composer_height = (draft.len() as u16).clamp(1, 6) + 2;
    let modal_height = if ui.approval.is_some() { 10 } else { 0 };
    let [header, body, modal, composer, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(modal_height),
        Constraint::Length(composer_height),
        Constraint::Length(2),
    ])
    .areas(f.area());

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" graffy chat ", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(format!(
                "· {} v{} · session {}",
                spec.graph.name,
                spec.graph.version,
                session.unwrap_or("not started")
            )),
        ])),
        header,
    );

    let lines = chat.lines(body.width);
    let overflow = (lines.len() as u16).saturating_sub(body.height);
    let back = chat.scroll_back.min(overflow);
    f.render_widget(Paragraph::new(lines).scroll((overflow - back, 0)), body);

    if ui.approval.is_some() {
        draw_modal(
            f,
            ui,
            modal,
            "Ctrl-A approve · Ctrl-R reject · ↑/↓ scroll · Ctrl-C stops the turn",
        );
    }

    // Show the tail of a long draft so the cursor line stays visible.
    let visible = (composer_height - 2) as usize;
    let tail = &draft[draft.len().saturating_sub(visible)..];
    f.render_widget(
        Paragraph::new(tail.join("\n")).block(Block::default().borders(Borders::ALL).title(
            if chat.busy {
                " message (a turn is running) "
            } else {
                " message "
            },
        )),
        composer,
    );
    if ui.approval.is_none() {
        let last = tail.last().map_or(0, |l| Span::raw(l.clone()).width()) as u16;
        f.set_cursor_position((
            composer.x + 1 + last.min(composer.width.saturating_sub(3)),
            composer.y + tail.len() as u16,
        ));
    }

    let status = match (&chat.hint, chat.busy) {
        (Some(hint), _) => format!(" {hint}"),
        (None, true) => format!(" working — {}", app.novice_line),
        (None, false) if back > 0 => {
            " ready — scrolled up; scroll down to return to the newest message".into()
        }
        (None, false) => " ready".into(),
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(status, Style::new().fg(Color::Yellow))),
            Line::from(Span::styled(
                " Enter send · wheel / ↑↓ / Ctrl-U Ctrl-D scroll · Tab run details · /new · /correct <text> · Ctrl-C stop turn / quit",
                Style::new().fg(Color::DarkGray),
            )),
        ]),
        footer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn type_text(chat: &mut ChatState, text: &str) {
        for c in text.chars() {
            assert_eq!(chat.on_key(key(KeyCode::Char(c))), Action::None);
        }
    }
    const CTRL_C: KeyEvent = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    #[test]
    fn letters_are_text_and_enter_submits_one_turn() {
        let mut chat = ChatState::default();
        type_text(&mut chat, "quit? yes, really");
        assert_eq!(
            chat.on_key(key(KeyCode::Enter)),
            Action::Submit {
                prompt: "quit? yes, really".into(),
                correction: false
            }
        );
        assert!(chat.input.is_empty());
        assert_eq!(chat.entries.len(), 1);
        // Blank input sends nothing.
        assert_eq!(chat.on_key(key(KeyCode::Enter)), Action::None);
    }

    #[test]
    fn enter_during_a_turn_keeps_the_draft_and_sends_nothing() {
        let mut chat = ChatState {
            busy: true,
            ..Default::default()
        };
        type_text(&mut chat, "next thought");
        assert_eq!(chat.on_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(chat.input, "next thought");
        assert!(chat.hint.is_some());
        assert!(chat.entries.is_empty());
    }

    #[test]
    fn ctrl_c_stops_a_running_turn_and_only_quits_when_idle() {
        let mut chat = ChatState {
            busy: true,
            ..Default::default()
        };
        assert_eq!(chat.on_key(CTRL_C), Action::Cancel);
        chat.busy = false;
        assert_eq!(chat.on_key(CTRL_C), Action::Quit);
    }

    #[tokio::test]
    async fn typing_cannot_decide_an_approval_only_ctrl_chords_can() {
        let (reply, mut decision) = tokio::sync::oneshot::channel();
        let mut ui = UiState::new(View::Inspect);
        ui.approval = Some(ApprovalPrompt {
            node_id: "release".into(),
            question: "Release?".into(),
            scroll: 0,
            reply,
        });
        // A sentence typed as the modal appears contains 'a', 'r', 'q', 'y'.
        for c in "a really quick yes".chars() {
            assert!(!approval_key(key(KeyCode::Char(c)), &mut ui));
        }
        assert!(!approval_key(key(KeyCode::Enter), &mut ui));
        assert!(decision.try_recv().is_err(), "nothing was decided");
        assert!(approval_key(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
            &mut ui
        ));
        assert_eq!(
            decision.await.unwrap(),
            ApprovalOutcome::ApprovedInput("ctrl-a".into())
        );
    }

    #[test]
    fn view_names_every_speaker_in_words_and_survives_tiny_terminals() {
        let spec =
            GraphSpec::from_toml_str(include_str!("../../../../graphs/conversation.default.toml"))
                .unwrap();
        let mut chat = ChatState::default();
        chat.push(Who::Human, "you", "hello");
        chat.push(Who::Ai, "graffy · reviewed response", "hi there");
        chat.push(Who::Note, "note", "no answer was released for this turn");
        let (app, ui) = (AppState::default(), UiState::new(View::Inspect));
        let render = |width, height| {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| draw_chat(f, &spec, None, &chat, &app, &ui))
                .unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
        };
        let screen = render(80, 24);
        for words in [
            "▸ you",
            "◂ graffy · reviewed response",
            "! note",
            "ready",
            "session not started",
        ] {
            assert!(screen.contains(words), "missing {words:?}");
        }
        // Resize down to almost nothing: must not panic.
        for (width, height) in [(20, 6), (4, 3), (1, 1)] {
            render(width, height);
        }
    }

    #[test]
    fn scrolling_works_without_page_keys_and_letters_stay_text() {
        let mut chat = ChatState::default();
        chat.on_key(key(KeyCode::Up));
        chat.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(chat.scroll_back, 11);
        chat.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        chat.on_key(key(KeyCode::Down));
        chat.on_key(key(KeyCode::Down)); // cannot scroll below the newest line
        assert_eq!(chat.scroll_back, 0);
        type_text(&mut chat, "jk");
        assert_eq!((chat.input.as_str(), chat.scroll_back), ("jk", 0));
        // A new message always returns the view to the bottom.
        chat.scroll(7);
        chat.push(Who::Note, "note", "hi");
        assert_eq!(chat.scroll_back, 0);
    }

    #[test]
    fn slash_commands_are_explicit() {
        let mut chat = ChatState::default();
        type_text(&mut chat, "/correct use ZEBRA");
        assert_eq!(
            chat.on_key(key(KeyCode::Enter)),
            Action::Submit {
                prompt: "use ZEBRA".into(),
                correction: true
            }
        );
        type_text(&mut chat, "/new");
        assert_eq!(chat.on_key(key(KeyCode::Enter)), Action::NewSession);
        // Anything else starting with '/' is an ordinary message.
        type_text(&mut chat, "/etc/hosts looks odd");
        assert!(matches!(
            chat.on_key(key(KeyCode::Enter)),
            Action::Submit {
                correction: false,
                ..
            }
        ));
    }

    #[test]
    fn paste_keeps_newlines_and_recorded_text_cannot_drive_the_terminal() {
        let mut chat = ChatState::default();
        chat.on_paste("line one\r\nline two");
        assert_eq!(chat.input, "line one\nline two");
        chat.push(Who::Ai, "graffy", "safe\x1b[2Jtext\x07");
        assert_eq!(chat.entries[0].text, "safe[2Jtext");
    }

    #[test]
    fn wrap_respects_width_including_long_and_wide_words() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap("one\n\ntwo", 10), ["one", "", "two"]);
        // Indentation is content: kept, and carried onto continuation lines.
        assert_eq!(wrap("    return value", 80), ["    return value"]);
        assert_eq!(wrap("  key: aa bb", 9), ["  key: aa", "  bb"]);
        assert_eq!(wrap("a  b", 80), ["a  b"]);
        assert!(wrap("          deep", 6).iter().all(|l| l.len() <= 6));
        // Wide characters take two cells each.
        assert!(
            wrap("日本語日本語", 4)
                .iter()
                .all(|l| Span::raw(l.clone()).width() <= 4)
        );
    }
}
