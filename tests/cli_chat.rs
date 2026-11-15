//! ADR-0010 acceptance journeys for `graffy chat`, through the real CLI and
//! the real runtime, using only the offline invoker.
use graffy_core::{
    journal::{JournalReader, wire},
    spec::GraphSpec,
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};
use wire::run_event::Event;

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "graffy-cli-chat-{}",
            graffy_core::id::RunId::generate()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn chat(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_graffy"))
            .env("GRAFFY_HOME", &self.0)
            .env_remove("GRAFFY_DATA_DIR")
            .arg("chat")
            .arg("--offline")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).ok();
        child.wait_with_output().unwrap()
    }
    /// Every run's manifest plus its event stream, oldest first.
    fn runs(&self) -> Vec<(wire::RunManifest, Vec<wire::RunEvent>)> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(self.0.join("runs"))
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|s| s == "journal"))
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|p| {
                let events = JournalReader::read_all(p).unwrap();
                let manifest = events
                    .iter()
                    .find_map(|e| match &e.event {
                        Some(Event::RunStarted(m)) => Some(m.clone()),
                        _ => None,
                    })
                    .unwrap();
                (manifest, events)
            })
            .collect()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn delivered(events: &[wire::RunEvent]) -> usize {
    events
        .iter()
        .filter(
            |e| matches!(&e.event, Some(Event::BoundaryEvent(b)) if b.visibility == "delivered"),
        )
        .count()
}
fn session_of(o: &Output) -> String {
    out(o)
        .lines()
        .find_map(|l| l.strip_prefix("resume this chat: graffy chat --session "))
        .expect("chat prints how to resume")
        .to_owned()
}

#[test]
fn follow_up_shares_one_session_and_identical_prompts_are_two_turns() {
    let h = Sandbox::new();
    let o = h.chat(
        &[],
        "the code word is ZEPHYR\n\nsame\nsame\n/quit\nNEVER-SENT\n",
    );
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    let runs = h.runs();
    assert_eq!(
        runs.len(),
        3,
        "blank lines and text after /quit send nothing"
    );

    // One session, three external turns: identical prompts are not retries.
    let sessions: BTreeSet<_> = runs.iter().map(|(m, _)| m.session_id.clone()).collect();
    let groups: BTreeSet<_> = runs
        .iter()
        .map(|(m, _)| m.attempt_group_id.clone())
        .collect();
    assert_eq!(sessions.len(), 1);
    assert_eq!(groups.len(), 3);
    assert_eq!(runs[0].0.run_kind, wire::RunKind::InitialAttempt as i32);
    assert_eq!(runs[1].0.run_kind, wire::RunKind::ExternalFollowup as i32);

    // The follow-up actually carried the earlier exchange, and each answer
    // shown has exactly one delivery receipt.
    assert!(out(&o).matches("ZEPHYR").count() >= 2);
    assert!(runs.iter().all(|(_, events)| delivered(events) == 1));
    assert_eq!(out(&o).matches("==== reviewed response ====").count(), 3);
    assert!(!out(&o).contains("NEVER-SENT"));
    // Step progress is on stderr, so stdout stays the conversation.
    assert!(err(&o).contains("step: draft"));
    assert!(!out(&o).contains("step: draft"));
}

#[test]
fn resume_restores_the_transcript_and_unknown_sessions_fail_loudly() {
    let h = Sandbox::new();
    let first = h.chat(&[], "remember MARIGOLD\n");
    let session = session_of(&first);

    let resumed = h.chat(&["--session", &session], "and now?\n");
    assert!(resumed.status.success(), "{}", err(&resumed));
    assert!(out(&resumed).contains("human> remember MARIGOLD"));
    assert_eq!(session_of(&resumed), session);
    let runs = h.runs();
    assert_eq!(runs.len(), 2, "restoring a transcript re-executes nothing");
    assert_eq!(runs[1].0.run_kind, wire::RunKind::ExternalFollowup as i32);

    let unknown = h.chat(&["--session", "ses_does_not_exist"], "hello\n");
    assert!(!unknown.status.success());
    assert!(err(&unknown).contains("unknown session"));
    assert_eq!(h.runs().len(), 2);
}

#[test]
fn a_failed_turn_is_not_an_answer_and_the_chat_carries_on() {
    let h = Sandbox::new();
    // An evidence floor the offline graph cannot meet: every turn fails review.
    let mut spec =
        GraphSpec::from_toml_str(include_str!("../graphs/conversation.default.toml")).unwrap();
    spec.policy.evidence.min_level = "L3".into();
    let path = h.0.join("strict.toml");
    std::fs::write(&path, spec.to_toml_string().unwrap()).unwrap();

    let o = h.chat(
        &["--graph", path.to_str().unwrap()],
        "/correct too early\nfirst\nsecond\n",
    );
    assert!(!o.status.success(), "unreleased turns exit nonzero");
    assert!(err(&o).contains("nothing to correct yet"));
    assert!(err(&o).contains("2 turn(s) released no answer"));
    assert!(out(&o).contains("no answer released"));
    assert!(!out(&o).contains("reviewed response"));
    let runs = h.runs();
    assert_eq!(
        runs.len(),
        2,
        "the second prompt still ran after the first failed"
    );
    assert!(runs.iter().all(|(_, events)| delivered(events) == 0));
}

const GATED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/graphs/conversation.gated.toml"
);

#[test]
fn piped_chat_lines_are_never_read_as_approvals() {
    let h = Sandbox::new();
    // Before the fix, "yes" was consumed as approval of the first answer.
    let o = h.chat(&["--graph", GATED], "hello\nyes\n");
    assert!(!o.status.success());
    assert!(err(&o).contains("abandoned, nothing released"));
    assert!(!out(&o).contains("reviewed response"));
    let runs = h.runs();
    assert_eq!(runs.len(), 2, "both lines are external turns");
    assert!(runs.iter().all(|(_, events)| delivered(events) == 0));
    assert!(runs.iter().all(|(_, events)| {
        !events.iter().any(
            |e| matches!(&e.event, Some(Event::Approval(a)) if a.decided_by.starts_with("human")),
        )
    }));
}

#[test]
fn resume_stays_on_the_sessions_graph() {
    let h = Sandbox::new();
    let first = h.chat(&["--graph", GATED], "hello\n");
    let resume = out(&first)
        .lines()
        .find(|l| l.starts_with("resume this chat:"))
        .unwrap()
        .to_owned();
    assert!(
        resume.ends_with(&format!(" --graph \"{GATED}\"")),
        "{resume}"
    );
    let session = resume.split_whitespace().nth(6).unwrap().to_owned();

    // No --graph on resume: the session's graph is restored, gate included.
    let resumed = h.chat(&["--session", &session], "again\n");
    assert!(
        err(&resumed).contains("abandoned, nothing released"),
        "{}",
        err(&resumed)
    );
    let runs = h.runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].0.graph_id, runs[1].0.graph_id);

    // An explicit different pin wins, and says so.
    let pinned = h.chat(
        &[
            "--session",
            &session,
            "--graph",
            "graffy.builtin.conversation",
        ],
        "third\n",
    );
    assert!(pinned.status.success(), "{}", err(&pinned));
    assert!(err(&pinned).contains("because --graph pins it"));
}
