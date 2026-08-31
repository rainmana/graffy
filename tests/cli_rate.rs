//! End-to-end CLI tests for the rating surface (P0.2).
use std::io::Write;
use std::process::{Command, Stdio};

fn graffy() -> Command {
    Command::new(env!("CARGO_BIN_EXE_graffy"))
}

fn run_prompt(home: &std::path::Path, prompt: &str, session: Option<&str>) -> String {
    let mut cmd = graffy();
    cmd.env("GRAFFY_HOME", home)
        .arg("run")
        .arg("graffy.builtin.conversation")
        .arg("--prompt")
        .arg(prompt)
        .arg("--offline");
    if let Some(s) = session {
        cmd.arg("--session").arg(s);
    }
    let out = cmd.output().expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    stdout
        .lines()
        .find(|l| l.starts_with("session :"))
        .map(|l| l.split_whitespace().last().unwrap().to_string())
        .expect("session id printed")
}

fn build_session(home: &std::path::Path, n: usize) -> String {
    let ses = run_prompt(home, "question 1", None);
    for i in 2..=n {
        run_prompt(home, &format!("question {i}"), Some(&ses));
    }
    ses
}

fn rate_with_input(
    home: &std::path::Path,
    session: &str,
    blinded: bool,
    input: &str,
) -> (String, bool) {
    let mut cmd = graffy();
    cmd.env("GRAFFY_HOME", home)
        .arg("rate")
        .arg(session)
        .arg("--rater")
        .arg("e2e");
    if blinded {
        cmd.arg("--blinded");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .ok();
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.success(),
    )
}

fn rating_journals(home: &std::path::Path) -> usize {
    std::fs::read_dir(home.join("runs"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains("-rating"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn blinded_rating_hides_internal_telemetry() {
    let home = std::env::temp_dir().join(format!("graffy-e2e-blind-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    let ses = build_session(&home, 1);
    // Unblinded: telemetry visible.
    let (plain, _) = rate_with_input(&home, &ses, false, "");
    assert!(
        plain.contains("internal orchestration telemetry"),
        "unblinded rating shows the telemetry class"
    );
    // Blinded: internal retry/failure/repair telemetry must NOT be shown —
    // it reveals the arm and primes the requested judgment.
    let (blind, _) = rate_with_input(&home, &ses, true, "");
    assert!(
        !blind.contains("internal orchestration telemetry"),
        "blinded rating must hide internal telemetry, got:\n{blind}"
    );
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn eof_abort_is_atomic_no_writes() {
    let home = std::env::temp_dir().join(format!("graffy-e2e-abort-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    let ses = build_session(&home, 5);
    let before = rating_journals(&home);
    // EOF at the very first prompt: nothing may be written.
    let (out, _) = rate_with_input(&home, &ses, false, "");
    assert!(out.contains("aborted"), "EOF aborts");
    assert_eq!(
        rating_journals(&home),
        before,
        "atomic abort: no rating journal"
    );
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn anchors_rendered_verbatim_and_audit_records_persisted() {
    let home = std::env::temp_dir().join(format!("graffy-e2e-audit-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    let ses = build_session(&home, 5);
    // accept window, H=3 (no citation needed), D=0, M=0.
    let (out, ok) = rate_with_input(&home, &ses, false, "\n3\n0\n0\n");
    assert!(ok, "rate completes: {out}");
    // Exact pinned anchor text (verbatim from the canonical rubric), not a
    // paraphrase:
    assert!(
        out.contains("H2 — adequate") && out.contains("repaired within the window"),
        "canonical H anchor table rendered verbatim"
    );
    assert!(
        out.contains("Measurement model (stated, not smuggled)"),
        "canonical D measurement model rendered verbatim"
    );
    // Audit records persisted with the samples.
    let runs = std::fs::read_dir(home.join("runs")).unwrap();
    let rating = runs
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.to_string_lossy().contains("-rating"))
        .expect("rating journal written");
    let events = graffy_core::journal::JournalReader::read_all(&rating).unwrap();
    use graffy_core::journal::wire::run_event::Event;
    let seg: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.event {
            Some(Event::Segmentation(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert!(
        seg.iter().any(|s| s.unit_id == "projection"
            && s.decision == "frozen"
            && !s.proposal_sha256.is_empty()),
        "frozen proposal persisted with hash"
    );
    assert!(
        seg.iter()
            .any(|s| s.unit_id == "window:1" && s.decision == "accepted"),
        "window acceptance persisted"
    );
    let samples: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.event {
            Some(Event::HrdmSampled(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(samples.len(), 3, "one sample per proxy (H, D, M)");
    for s in &samples {
        assert!(
            !s.evidence_refs.is_empty(),
            "every score cites journal refs"
        );
        assert!(s.evidence_refs[0].starts_with("journal://"));
    }
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn excluded_units_persist_even_when_nothing_is_scored() {
    let home = std::env::temp_dir().join(format!("graffy-e2e-excl-{}", std::process::id()));
    std::fs::remove_dir_all(&home).ok();
    let ses = build_session(&home, 5);
    // Exclude the only window (with required note); nothing gets scored.
    let (out, ok) = rate_with_input(&home, &ses, false, "x\nsegmentation looks wrong\n");
    assert!(ok, "rate completes: {out}");
    let runs = std::fs::read_dir(home.join("runs")).unwrap();
    let rating = runs
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.to_string_lossy().contains("-rating"))
        .expect("audit journal written even though nothing was scored");
    let events = graffy_core::journal::JournalReader::read_all(&rating).unwrap();
    use graffy_core::journal::wire::run_event::Event;
    let excluded = events.iter().any(|e| {
        matches!(
            &e.event,
            Some(Event::Segmentation(s))
                if s.decision == "excluded" && s.correction.contains("segmentation looks wrong")
        )
    });
    assert!(excluded, "exclusion + structured correction persisted");
    let sample_count = events
        .iter()
        .filter(|e| matches!(&e.event, Some(Event::HrdmSampled(_))))
        .count();
    assert_eq!(sample_count, 0, "no samples — audit only");
    std::fs::remove_dir_all(&home).ok();
}
