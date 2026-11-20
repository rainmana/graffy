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
        self.chat_env(args, input, &[])
    }
    fn chat_env(&self, args: &[&str], input: &str, env: &[(&str, &str)]) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_graffy"))
            .env("GRAFFY_HOME", &self.0)
            .env_remove("GRAFFY_DATA_DIR")
            // The decider is configured only by what a test passes in.
            .env_remove("GRAFFY_DECIDER_URL")
            .env_remove("GRAFFY_DECIDER_MODEL")
            .env_remove("GRAFFY_DECIDER_API_KEY")
            .env_remove("GRAFFY_ROUTER_MIN_CONFIDENCE")
            // So is the context limit: unset, the graph's budget decides.
            .env_remove("GRAFFY_CONTEXT_MAX_BYTES")
            .envs(env.iter().copied())
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

#[test]
fn routed_chat_runs_the_router_as_its_own_journaled_run() {
    let h = Sandbox::new();
    let o = h.chat(
        &["--route"],
        "hello\nwhat are the pros and cons of renting?\nhelp me decide, step by step\n/chat pros and cons, but just chat\n",
    );
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    let runs = h.runs();
    let (routing, turns): (Vec<_>, Vec<_>) = runs
        .iter()
        .partition(|(m, _)| m.run_kind == wire::RunKind::Routing as i32);
    // Four messages are four exchanges; three were routed, `/chat` skipped it.
    assert_eq!((routing.len(), turns.len()), (3, 4));
    let decision = |events: &[wire::RunEvent]| graffy_core::route::decision_from(events).unwrap();

    // 1. No phrase: the conversation floor, and the reason is said plainly.
    assert!(decision(&routing[0].1).fallback);
    // 2. One graph's phrase: that graph runs, linked to the routing run.
    let chosen = decision(&routing[1].1);
    assert_eq!(
        chosen.chosen_graph_id,
        "graffy.builtin.reasoning.decision-framework"
    );
    let routed_turn = turns
        .iter()
        .find(|(m, _)| m.routed_by_run_id == routing[1].0.run_id)
        .expect("the target run names its routing run");
    assert_eq!(routed_turn.0.graph_id, chosen.chosen_graph_id);
    assert!(out(&o).contains(
        "routing : running 'Decision Framework' because your message says \"pros and cons\""
    ));
    // 3. Two graphs' phrases: ambiguous, so nothing is guessed.
    let ambiguous = decision(&routing[2].1);
    assert!(ambiguous.fallback && ambiguous.matched.len() == 2);
    // 4. `/chat` is the human override: no routing run, plain conversation.
    let plain = turns.last().unwrap();
    assert!(plain.0.routed_by_run_id.is_empty());
    assert_eq!(plain.0.graph_id, "graffy.builtin.conversation");

    // Routing runs are not conversation turns: resuming shows four human
    // messages, not seven, and one session holds all four exchanges.
    let session = out(&o)
        .lines()
        .find_map(|l| l.strip_prefix("resume this chat: graffy chat --session "))
        .unwrap()
        .to_owned();
    assert!(session.ends_with(" --route"), "{session}");
    let id = session.trim_end_matches(" --route");
    let resumed = h.chat(&["--session", id, "--route"], "");
    assert_eq!(out(&resumed).matches("human> ").count(), 4);

    // A pin and the router cannot both choose the graph.
    let both = h.chat(&["--route", "--graph", "graffy.builtin.conversation"], "");
    assert!(!both.status.success());
}

/// A local stand-in for a decision-model server: answers every
/// `POST /v1/systemone` with the same choice. Returns its base URL.
fn decider_fixture(choice: &'static str, confidence: f64) -> String {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap_or(0);
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                let done = text.split_once("\r\n\r\n").is_some_and(|(head, sent)| {
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    sent.len() >= length
                });
                if done || read == 0 {
                    break;
                }
            }
            let body = format!(
                r#"{{"model":"fixture-decider-1","answers":{{"route":{{"type":"choice","choice":"{choice}","confidence":{confidence},"probabilities":{{"{choice}":{confidence}}}}}}},"usage":{{"input_tokens":50,"output_tokens":1}}}}"#
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .ok();
        }
    });
    url
}

#[test]
fn decider_routing_follows_the_model_only_above_the_configured_threshold() {
    const DECISION: &str = "graffy.builtin.reasoning.decision-framework";
    // No trigger phrase in this message: only the decision model can route it.
    let message = "Should I rent or buy a flat?\n";

    let h = Sandbox::new();
    let url = decider_fixture(DECISION, 0.97);
    let env = [
        ("GRAFFY_DECIDER_URL", url.as_str()),
        ("GRAFFY_ROUTER_MIN_CONFIDENCE", "0.9"),
    ];
    let o = h.chat_env(&["--route=decider"], message, &env);
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    // The human is told, before anything is sent, where messages go.
    let endpoint = url.trim_start_matches("http://");
    assert!(out(&o).contains(&format!("every routed message is SENT to {endpoint}")));
    assert!(out(&o).contains("chose it with confidence 0.97 (your threshold 0.90)"));
    let runs = h.runs();
    let routing = runs
        .iter()
        .find(|(m, _)| m.run_kind == wire::RunKind::Routing as i32)
        .unwrap();
    let decision = graffy_core::route::decision_from(&routing.1).unwrap();
    assert_eq!(
        (decision.router.as_str(), decision.chosen_graph_id.as_str()),
        ("decider", DECISION)
    );
    assert_eq!(decision.decided_by_model, "fixture-decider-1");
    assert_eq!(
        (decision.confidence, decision.min_confidence),
        (Some(0.97), Some(0.9))
    );
    assert!(
        runs.iter()
            .any(|(m, _)| m.graph_id == DECISION && m.routed_by_run_id == routing.0.run_id)
    );

    // The same answer under a stricter threshold is not followed.
    let h = Sandbox::new();
    let strict = [
        ("GRAFFY_DECIDER_URL", url.as_str()),
        ("GRAFFY_ROUTER_MIN_CONFIDENCE", "0.99"),
    ];
    let o = h.chat_env(&["--route=decider"], message, &strict);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("below your threshold 0.99"));
    assert!(h.runs().iter().all(|(m, _)| m.graph_id != DECISION));
}

#[test]
fn decider_routing_refuses_to_start_on_guessed_or_missing_configuration() {
    let h = Sandbox::new();
    let none = h.chat_env(&["--route=decider"], "hi\n", &[]);
    assert!(!none.status.success());
    assert!(err(&none).contains("GRAFFY_DECIDER_URL is not set"));

    // No threshold: graffy ships no default.
    let url = decider_fixture("graffy.builtin.conversation", 0.5);
    let no_threshold = h.chat_env(
        &["--route=decider"],
        "hi\n",
        &[("GRAFFY_DECIDER_URL", url.as_str())],
    );
    assert!(!no_threshold.status.success());
    assert!(err(&no_threshold).contains("graffy ships no default"));

    // An unreachable decider fails the turn loudly; nothing is routed by guess.
    let dead = [
        ("GRAFFY_DECIDER_URL", "http://127.0.0.1:9"),
        ("GRAFFY_ROUTER_MIN_CONFIDENCE", "0.9"),
    ];
    let unreachable = h.chat_env(&["--route=decider"], "hi\n", &dead);
    assert!(!unreachable.status.success());
    assert!(err(&unreachable).contains("routing failed"));
    assert!(!out(&unreachable).contains("reviewed response"));
    assert!(
        h.runs()
            .iter()
            .all(|(m, _)| m.run_kind == wire::RunKind::Routing as i32)
    );
}

/// The text a draft node was sent, recovered from the offline echo's answer.
fn draft_inputs(events: &[wire::RunEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match &e.event {
            Some(Event::IuRecorded(iu))
                if iu.attributes.get("role").map(String::as_str) == Some("draft") =>
            {
                iu.payload_text
                    .strip_prefix("[offline echo — no real model was consulted]\n")
                    .map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_long_chat_sends_a_bounded_disclosed_history_and_keeps_the_opening_constraint() {
    const LIMIT: usize = 2000;
    let h = Sandbox::new();
    // The offline echo repeats its whole input, so every answer is longer
    // than the last: without a policy this chat's input grows without bound.
    let mut input = String::from("RULE-OPAL: answer in metric units only\n");
    for n in 0..9 {
        input.push_str(&format!("filler question {n}\n"));
    }
    let o = h.chat_env(&[], &input, &[("GRAFFY_CONTEXT_MAX_BYTES", "2000")]);
    assert!(o.status.success(), "{}\n{}", out(&o), err(&o));
    let runs = h.runs();
    assert_eq!(runs.len(), 10, "every turn still runs and releases");
    assert!(runs.iter().all(|(_, events)| delivered(events) == 1));

    let mut bounded_turns = 0;
    for (_, events) in &runs {
        let inputs = draft_inputs(events);
        assert_eq!(inputs.len(), 1);
        assert!(
            inputs[0].len() <= LIMIT,
            "a draft was sent {} bytes; the limit is {LIMIT}",
            inputs[0].len()
        );
        assert!(
            inputs[0].contains("RULE-OPAL: answer in metric units only"),
            "the opening constraint was trimmed away"
        );
        // A trimmed turn says so to the model, and the journal keeps that line.
        let disclosed = events.iter().any(|e| {
            matches!(&e.event, Some(Event::IuRecorded(iu))
                if iu.attributes.get("actor").map(String::as_str) == Some("graffy")
                    && iu.payload_text.starts_with("[context policy]"))
        });
        assert_eq!(disclosed, inputs[0].contains("graffy: [context policy]"));
        bounded_turns += usize::from(disclosed);
    }
    assert!(bounded_turns > 0, "the chat never grew past the limit");
    // The human is told once per trimmed turn, where warnings go.
    assert_eq!(
        err(&o)
            .matches("warning: context policy: the model was sent")
            .count(),
        bounded_turns
    );
    assert!(err(&o).contains("set by GRAFFY_CONTEXT_MAX_BYTES"));
}

#[test]
fn a_turn_too_large_for_the_limit_stops_loudly_and_a_bad_limit_never_starts_a_chat() {
    let h = Sandbox::new();
    let o = h.chat_env(
        &[],
        "RULE-OPAL: answer in metric units only\nand then?\n",
        &[("GRAFFY_CONTEXT_MAX_BYTES", "60")],
    );
    assert!(!o.status.success(), "a refused turn is not a success");
    assert!(
        err(&o).contains("turn not started: context policy: turn not sent"),
        "{}",
        err(&o)
    );
    assert!(err(&o).contains("No model was called"));
    assert_eq!(h.runs().len(), 1, "the refused turn ran nothing");

    for bad in ["0", "lots", "-5"] {
        let o = h.chat_env(&[], "hello\n", &[("GRAFFY_CONTEXT_MAX_BYTES", bad)]);
        assert!(!o.status.success());
        assert!(
            err(&o).contains("GRAFFY_CONTEXT_MAX_BYTES must be a positive whole number"),
            "{}",
            err(&o)
        );
    }
    assert_eq!(h.runs().len(), 1, "a bad limit sends nothing");
}
