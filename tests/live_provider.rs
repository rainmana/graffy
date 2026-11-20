//! Live-provider end-to-end smoke (issue #32) — see `docs/live-smoke.md`.
//!
//! Everything else in the suite runs on `OfflineEcho` or in-process fakes, so
//! the path a real user takes — `GRAFFY_MODEL_*` bindings, a provider key, a
//! real HTTP client, real usage numbers — was only ever checked by hand. These
//! tests drive that path through the real `graffy` binary and the real
//! executor, then read the journal the run wrote.
//!
//! Two tests are opt-in (`#[ignore]`) because they need a model to talk to:
//!
//! ```text
//! cargo test --test live_provider live_hosted -- --ignored --nocapture
//! cargo test --test live_provider live_ollama -- --ignored --nocapture
//! ```
//!
//! They read the same environment a real run reads — nothing is remapped and
//! no model name lives in this file. Bind every tier the default conversation
//! can ask for (`GRAFFY_MODEL_BALANCED`, plus `GRAFFY_MODEL_FRONTIER` for the
//! escalation a REVISE verdict triggers) to `provider:model`, and set the
//! provider's key (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`
//! or `VENICE_API_KEY`; Ollama needs none and honors `OLLAMA_API_BASE_URL`).
//! An opt-in test that is not configured FAILS with instructions — a green
//! result must mean a provider really answered.
//!
//! The remaining tests are hermetic and run in the normal offline gate. They
//! pin the preflight rules, prove that an unreachable provider ends the run
//! loudly with a journaled failure and no released answer, and run the live
//! acceptance check against a loopback stub so it cannot rot unexecuted. The
//! stub proves plumbing, not provider compatibility.

use graffy_core::{
    exec::sha256_hex,
    journal::{JournalReader, wire},
    spec::GraphSpec,
};
use graffy_providers::{ProviderKind, TierBinding, bindings_from_env};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use wire::run_event::Event;

const GRAPH_ID: &str = "graffy.builtin.conversation";
/// Answerable from the request alone, so the verify gate is judging a
/// conversational reply rather than an external factual claim.
const PROMPT: &str = "Reply with one short sentence confirming that you received this message.";
const RESPONSE_BANNER: &str = "================ reviewed response";

/// Which provider family a live test claims to have covered. The test name is
/// the claim, so a run bound to the other family must fail, not pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivePath {
    /// A keyed HTTPS API: Anthropic, or an OpenAI-compatible provider.
    Hosted,
    /// A local (or self-hosted) Ollama server; no key.
    Ollama,
}

impl LivePath {
    /// Exhaustive on purpose: a new provider must be placed in a family here
    /// before either live test will accept it.
    fn of(provider: ProviderKind) -> Self {
        match provider {
            ProviderKind::Anthropic
            | ProviderKind::OpenAi
            | ProviderKind::OpenRouter
            | ProviderKind::Venice => Self::Hosted,
            ProviderKind::Ollama => Self::Ollama,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Hosted => "hosted (anthropic | openai | openrouter | venice)",
            Self::Ollama => "ollama",
        }
    }

    fn example_provider(self) -> &'static str {
        match self {
            Self::Hosted => "anthropic",
            Self::Ollama => "ollama",
        }
    }
}

/// Every tier a run of `spec` can ask a model for: each node's declared tier,
/// plus the rungs above it on the routing ladder, because a REVISE verdict
/// escalates up the ladder (`resolve_tier` in graffy-core's executor). A tier
/// left unbound would turn a judge's REVISE into an `UnboundTier` failure that
/// says nothing about the provider under test.
fn tiers_a_run_can_ask_for(spec: &GraphSpec) -> BTreeSet<String> {
    let ladder = &spec.policy.routing.ladder;
    let mut tiers = BTreeSet::new();
    for base in spec.nodes.iter().filter_map(|n| n.model_tier.as_ref()) {
        tiers.insert(base.clone());
        if let Some(position) = ladder.iter().position(|rung| rung == base) {
            tiers.extend(ladder[position + 1..].iter().cloned());
        }
    }
    tiers
}

/// Decide whether the environment's bindings let `path` be tested honestly.
/// Returns the `provider:model` targets a model call may lawfully land on, or
/// every problem at once with the fix.
fn preflight(
    path: LivePath,
    required_tiers: &BTreeSet<String>,
    bindings: &HashMap<String, TierBinding>,
) -> Result<BTreeSet<String>, String> {
    let mut problems = Vec::new();
    let mut targets = BTreeSet::new();
    for tier in required_tiers {
        let var = format!("GRAFFY_MODEL_{}", tier.to_ascii_uppercase());
        match bindings.get(tier) {
            None => problems.push(format!("{var} is not set (or is not 'provider:model')")),
            Some(binding) if LivePath::of(binding.provider) != path => problems.push(format!(
                "{var} is bound to '{}', which this test does not cover",
                binding.provider.name()
            )),
            Some(binding) => {
                targets.insert(format!("{}:{}", binding.provider.name(), binding.model));
            }
        }
    }
    if problems.is_empty() {
        return Ok(targets);
    }
    let example = required_tiers
        .iter()
        .map(|tier| {
            format!(
                "GRAFFY_MODEL_{}={}:<model>",
                tier.to_ascii_uppercase(),
                path.example_provider()
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    Err(format!(
        "live smoke for the {} path is not configured:\n  - {}\n\
         bind every tier the default conversation can ask for, e.g.\n  {example}\n\
         plus that provider's key (Ollama needs none). See docs/live-smoke.md.\n\
         This test never falls back to the offline echo: unconfigured means failed.",
        path.describe(),
        problems.join("\n  - "),
    ))
}

/// A throwaway graffy home. Kept on disk when the test fails, so the journal
/// that explains the failure can still be replayed.
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "graffy-live-smoke-{}",
            graffy_core::id::RunId::generate()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn cmd(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_graffy"));
        command
            .env("GRAFFY_HOME", &self.0)
            .env_remove("GRAFFY_DATA_DIR")
            .stdin(Stdio::null());
        command
    }
    fn journals(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.0.join("runs"))
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "journal"))
            .collect()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("graffy home kept for inspection: {}", self.0.display());
        } else {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
}

fn transcript(out: &Output) -> String {
    format!(
        "exit: {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn model_calls(events: &[wire::RunEvent]) -> Vec<&wire::ModelCallRecord> {
    events
        .iter()
        .filter_map(|event| match &event.event {
            Some(Event::ModelCall(call)) => Some(call),
            _ => None,
        })
        .collect()
}

/// Not necessarily the last frame: a delivered answer appends its output
/// receipt after the run has finished.
fn run_finished(events: &[wire::RunEvent]) -> Option<&wire::RunFinished> {
    events.iter().rev().find_map(|event| match &event.event {
        Some(Event::RunFinished(finished)) => Some(finished),
        _ => None,
    })
}

fn released_answers(events: &[wire::RunEvent]) -> Vec<&wire::BoundaryEventRecord> {
    events
        .iter()
        .filter_map(|event| match &event.event {
            Some(Event::BoundaryEvent(boundary)) if boundary.visibility == "released" => {
                Some(boundary)
            }
            _ => None,
        })
        .collect()
}

/// One real turn of the default conversation against whatever the environment
/// binds, exactly as a user's shell would run it.
fn live_smoke(path: LivePath) {
    let spec = GraphSpec::from_toml_str(graffy_graphs::DEFAULT_CONVERSATION_TOML).unwrap();
    let targets = match preflight(path, &tiers_a_run_can_ask_for(&spec), &bindings_from_env()) {
        Ok(targets) => targets,
        Err(why) => panic!("{why}"),
    };
    let home = Sandbox::new();
    let out = home
        .cmd()
        .args(["run", GRAPH_ID, "--prompt", PROMPT])
        .output()
        .unwrap();
    let usage = assert_released_run(path, &targets, &home, &out);
    eprintln!(
        "live smoke [{}]: {} call(s) to {targets:?}; {} in / {} out tokens as reported by \
         the provider; answer released",
        path.describe(),
        usage.calls,
        usage.input_tokens,
        usage.output_tokens
    );
}

/// What the journaled provider calls of one run add up to.
struct Usage {
    calls: usize,
    input_tokens: u64,
    output_tokens: u64,
}

/// The acceptance check for one non-offline turn. Transport facts are checked
/// first so a failure names the layer that broke: configuration, the provider
/// call, usage accounting, or the graph's own verify gate.
fn assert_released_run(
    path: LivePath,
    targets: &BTreeSet<String>,
    home: &Sandbox,
    out: &Output,
) -> Usage {
    let spec_text = graffy_graphs::DEFAULT_CONVERSATION_TOML;
    let said = transcript(out);

    let journals = home.journals();
    assert_eq!(
        journals.len(),
        1,
        "one turn without --retry must write exactly one journal; a run that never \
         started (missing key, bad binding) writes none\n{said}"
    );
    let events = JournalReader::read_all(&journals[0]).unwrap();
    match &events[0].event {
        Some(Event::RunStarted(manifest)) => {
            assert_eq!(manifest.graph_id, GRAPH_ID);
            assert_eq!(
                manifest.spec_sha256,
                sha256_hex(spec_text.as_bytes()),
                "the registry ran a different spec than the one this test preflighted"
            );
        }
        other => panic!("a journal must open with its manifest, found {other:?}"),
    }

    let calls = model_calls(&events);
    assert!(
        !calls.is_empty(),
        "no provider call completed, so nothing about the live path was exercised\n{said}"
    );
    for call in &calls {
        let target = format!("{}:{}", call.provider, call.model);
        assert!(
            targets.contains(&target),
            "node '{}' was answered by {target}, which is not one of the bound {} targets {targets:?}",
            call.node_id,
            path.describe()
        );
        // Budgets are enforced on what the provider reports. A zero here means
        // the whole-turn token budget undercounts on this provider — a finding
        // to record, not something to relax in this test.
        assert!(
            call.input_tokens > 0 && call.output_tokens > 0,
            "{target} reported {} input / {} output tokens for node '{}' ({}); graffy's \
             budgets rely on real usage",
            call.input_tokens,
            call.output_tokens,
            call.node_id,
            call.purpose
        );
    }

    let finished = run_finished(&events)
        .unwrap_or_else(|| panic!("the journal must record run_finished\n{said}"));
    let input: u64 = calls.iter().map(|call| call.input_tokens).sum();
    let output: u64 = calls.iter().map(|call| call.output_tokens).sum();
    assert_eq!(
        (finished.total_input_tokens, finished.total_output_tokens),
        (input, output),
        "run totals must equal the sum of the journaled provider calls"
    );
    assert_eq!(
        wire::RunStatus::try_from(finished.status),
        Ok(wire::RunStatus::Succeeded),
        "the provider answered {} call(s) ({input} in / {output} out tokens), but the graph \
         released no answer. That is the graph's verify gate or budget speaking, not the \
         transport: replay the kept journal to read the judge's reason.\n{said}",
        calls.len()
    );
    assert!(out.status.success(), "a succeeded run must exit 0\n{said}");
    let released = released_answers(&events);
    assert_eq!(released.len(), 1, "exactly one answer is released per turn");
    assert!(!released[0].content.trim().is_empty());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(RESPONSE_BANNER) && stdout.contains(released[0].content.trim()),
        "the released answer must be the one printed\n{said}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(&event.event, Some(Event::BoundaryEvent(b)) if b.visibility == "delivered"))
            .count(),
        1,
        "printing the answer must leave exactly one output receipt"
    );
    Usage {
        calls: calls.len(),
        input_tokens: input,
        output_tokens: output,
    }
}

#[test]
#[ignore = "live: needs GRAFFY_MODEL_BALANCED/_FRONTIER bound to a hosted provider plus its API key (docs/live-smoke.md)"]
fn live_hosted_provider_releases_a_journaled_answer() {
    live_smoke(LivePath::Hosted);
}

#[test]
#[ignore = "live: needs GRAFFY_MODEL_BALANCED/_FRONTIER bound to ollama and a reachable Ollama server (docs/live-smoke.md)"]
fn live_ollama_releases_a_journaled_answer() {
    live_smoke(LivePath::Ollama);
}

// ---------------------------------------------------------------------------
// Hermetic: these run in the required offline gate.
// ---------------------------------------------------------------------------

fn binding(provider: ProviderKind, model: &str) -> TierBinding {
    TierBinding {
        provider,
        model: model.to_owned(),
    }
}

fn tiers(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn default_conversation_needs_balanced_and_the_rung_it_escalates_to() {
    let spec = GraphSpec::from_toml_str(graffy_graphs::DEFAULT_CONVERSATION_TOML).unwrap();
    // docs/live-smoke.md tells people to bind exactly these two; keep it true.
    assert_eq!(
        tiers_a_run_can_ask_for(&spec),
        tiers(&["balanced", "frontier"])
    );
}

#[test]
fn unconfigured_live_smoke_is_a_failure_with_instructions() {
    let required = tiers(&["balanced", "frontier"]);
    for path in [LivePath::Hosted, LivePath::Ollama] {
        let why = preflight(path, &required, &HashMap::new()).unwrap_err();
        for expected in [
            "GRAFFY_MODEL_BALANCED is not set",
            "GRAFFY_MODEL_FRONTIER is not set",
            "docs/live-smoke.md",
            "never falls back to the offline echo",
        ] {
            assert!(why.contains(expected), "missing '{expected}' in:\n{why}");
        }
    }

    // One tier bound is not enough: a REVISE verdict would hit the unbound rung.
    let mut bindings = HashMap::new();
    bindings.insert(
        "balanced".to_owned(),
        binding(ProviderKind::Ollama, "placeholder"),
    );
    let why = preflight(LivePath::Ollama, &required, &bindings).unwrap_err();
    assert!(why.contains("GRAFFY_MODEL_FRONTIER is not set"));
    assert!(!why.contains("GRAFFY_MODEL_BALANCED is"));
}

#[test]
fn a_live_test_refuses_bindings_from_the_other_family() {
    let required = tiers(&["balanced", "frontier"]);
    let mut bindings = HashMap::new();
    bindings.insert(
        "balanced".to_owned(),
        binding(ProviderKind::Ollama, "placeholder-a"),
    );
    bindings.insert(
        "frontier".to_owned(),
        binding(ProviderKind::OpenRouter, "placeholder-b"),
    );
    // Mixed families: neither test may claim the run.
    let hosted = preflight(LivePath::Hosted, &required, &bindings).unwrap_err();
    assert!(hosted.contains("GRAFFY_MODEL_BALANCED is bound to 'ollama'"));
    let ollama = preflight(LivePath::Ollama, &required, &bindings).unwrap_err();
    assert!(ollama.contains("GRAFFY_MODEL_FRONTIER is bound to 'openrouter'"));

    bindings.insert(
        "balanced".to_owned(),
        binding(ProviderKind::Anthropic, "placeholder-a"),
    );
    // A tier the graph never asks for is not this test's business.
    bindings.insert(
        "fast".to_owned(),
        binding(ProviderKind::Ollama, "placeholder-c"),
    );
    assert_eq!(
        preflight(LivePath::Hosted, &required, &bindings).unwrap(),
        tiers(&["anthropic:placeholder-a", "openrouter:placeholder-b"])
    );
}

/// A non-offline `graffy run` whose provider configuration is only what the
/// test binds: a developer's real bindings, output caps or proxy settings must
/// not change what a hermetic test exercises.
fn hermetic_run(home: &Sandbox, ollama_base_url: &str, bindings: &[(&str, &str)]) -> Output {
    let mut command = home.cmd();
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GRAFFY_MODEL_")
            || upper.starts_with("GRAFFY_MAX_OUTPUT_TOKENS_")
            || upper.ends_with("_PROXY")
        {
            command.env_remove(&name);
        }
    }
    command
        .envs(bindings.iter().copied())
        .env("OLLAMA_API_BASE_URL", ollama_base_url)
        .env("NO_PROXY", "127.0.0.1")
        .args(["run", GRAPH_ID, "--prompt", PROMPT])
        .output()
        .unwrap()
}

/// The first test in the suite to reach the real provider adapter through the
/// CLI without `--offline`: bindings parsed from the environment, a real HTTP
/// client dialing out, and the executor recording what happened. The "provider"
/// is a loopback listener that hangs up on every connection, so no model is
/// needed and nothing leaves the machine.
#[test]
fn unreachable_provider_fails_loudly_and_releases_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let dialed = Arc::new(AtomicUsize::new(0));
    let counter = dialed.clone();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(connection);
        }
    });

    let home = Sandbox::new();
    // Not a model name: nothing ever serves it.
    let target = "ollama:no-model-is-served-here";
    let out = hermetic_run(&home, &base_url, &[("GRAFFY_MODEL_BALANCED", target)]);
    let said = transcript(&out);

    assert!(!out.status.success(), "{said}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("provider error"),
        "the failure must name the provider layer\n{said}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tier balanced") && stdout.contains(target),
        "the live path announces its bindings before running\n{said}"
    );
    assert!(!stdout.contains("OFFLINE ECHO"), "{said}");
    assert!(!stdout.contains(RESPONSE_BANNER), "{said}");
    assert!(
        dialed.load(Ordering::SeqCst) >= 1,
        "the real provider client must have dialed the configured base URL\n{said}"
    );

    let journals = home.journals();
    assert_eq!(journals.len(), 1, "a failed run is still journaled\n{said}");
    let events = JournalReader::read_all(&journals[0]).unwrap();
    let finished = run_finished(&events).expect("the journal must record run_finished");
    assert_eq!(finished.status, wire::RunStatus::Failed as i32);
    // Nothing answered, so nothing may be recorded as answered or spent.
    assert!(model_calls(&events).is_empty());
    assert_eq!(
        (
            finished.total_input_tokens,
            finished.total_output_tokens,
            finished.total_usd
        ),
        (0, 0, 0.0)
    );
    assert!(released_answers(&events).is_empty());
}

const STUB_INPUT_TOKENS: u64 = 12;
const STUB_OUTPUT_TOKENS: u64 = 3;
const STUB_REPLY: &str = "PASS. Stub reply: the message was received.";

/// A loopback stand-in for Ollama: answers every non-streaming `/api/chat`
/// with one canned reply in the response shape rig-core 0.42 parses, and keeps
/// the request bodies it was sent. The reply opens with PASS so the verify
/// node's reviewer call reads it as a pass without the stub having to
/// recognise graffy's prompts.
///
/// It is NOT evidence that graffy works with Ollama — only the ignored live
/// test can show that. It exists so the acceptance check the live tests rely
/// on runs in every offline gate instead of rotting unexecuted.
fn spawn_stub_ollama() -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let mut connection = connection.unwrap();
            let request = read_json_request(&mut connection);
            let reply = serde_json::json!({
                "model": request["model"],
                "created_at": "1970-01-01T00:00:00Z",
                "message": { "role": "assistant", "content": STUB_REPLY },
                "done": true,
                "done_reason": "stop",
                "total_duration": 1,
                "load_duration": 1,
                "prompt_eval_count": STUB_INPUT_TOKENS,
                "prompt_eval_duration": 1,
                "eval_count": STUB_OUTPUT_TOKENS,
                "eval_duration": 1,
            })
            .to_string();
            seen.lock().unwrap().push(request);
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
    });
    (base_url, requests)
}

/// Read one HTTP request with a `Content-Length` JSON body.
fn read_json_request(connection: &mut TcpStream) -> serde_json::Value {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(head_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
            let length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .expect("the stub only understands Content-Length bodies")
                .trim()
                .parse()
                .unwrap();
            let body_start = head_end + 4;
            if raw.len() >= body_start + length {
                return serde_json::from_slice(&raw[body_start..body_start + length]).unwrap();
            }
        }
        let read = connection.read(&mut chunk).unwrap();
        assert!(read > 0, "client hung up mid-request");
        raw.extend_from_slice(&chunk[..read]);
    }
}

/// The success half of the live path, hermetically: environment bindings and
/// the per-tier output cap reach the wire, the reply comes back through rig's
/// real Ollama client, and the usage the server reported is what the journal
/// and the run totals record.
#[test]
fn stub_provider_run_passes_the_live_acceptance_check() {
    let (base_url, requests) = spawn_stub_ollama();
    let home = Sandbox::new();
    let out = hermetic_run(
        &home,
        &base_url,
        &[
            // Placeholders, not model names: the stub answers to anything.
            ("GRAFFY_MODEL_BALANCED", "ollama:stub-balanced"),
            ("GRAFFY_MODEL_FRONTIER", "ollama:stub-frontier"),
            ("GRAFFY_MAX_OUTPUT_TOKENS_BALANCED", "321"),
        ],
    );
    let mut bindings = HashMap::new();
    for (tier, model) in [("balanced", "stub-balanced"), ("frontier", "stub-frontier")] {
        bindings.insert(tier.to_owned(), binding(ProviderKind::Ollama, model));
    }
    let targets = preflight(
        LivePath::Ollama,
        &tiers(&["balanced", "frontier"]),
        &bindings,
    )
    .unwrap();

    let usage = assert_released_run(LivePath::Ollama, &targets, &home, &out);

    // draft + verify, both on the balanced tier: the stub reviewer passes, so
    // nothing escalates to the frontier binding.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(usage.calls, 2);
    assert_eq!(usage.input_tokens, 2 * STUB_INPUT_TOKENS);
    assert_eq!(usage.output_tokens, 2 * STUB_OUTPUT_TOKENS);
    for request in requests.iter() {
        assert_eq!(request["model"], "stub-balanced");
        assert_eq!(request["stream"], false);
        assert_eq!(
            request["options"]["num_predict"], 321,
            "GRAFFY_MAX_OUTPUT_TOKENS_BALANCED must reach the provider as the per-call cap"
        );
    }
}
