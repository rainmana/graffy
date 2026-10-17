//! Release acceptance journeys through the real CLI, using only offline models.
use graffy_core::{
    journal::{JournalReader, wire},
    spec::{EdgeSpec, GraphSpec},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use wire::run_event::Event;

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "graffy-cli-release-{}",
            graffy_core::id::RunId::generate()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_graffy"));
        c.env("GRAFFY_HOME", &self.0).env_remove("GRAFFY_DATA_DIR");
        c
    }
    fn run(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .cmd()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).ok();
        child.wait_with_output().unwrap()
    }
    fn journals(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.0.join("runs"))
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|s| s == "journal"))
            .collect()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn session(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}\n{}",
        text(out),
        String::from_utf8_lossy(&out.stderr)
    );
    text(out)
        .lines()
        .find(|s| s.starts_with("session :"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .into()
}
fn save_spec(home: &Sandbox, s: &GraphSpec) -> PathBuf {
    let p = home.0.join("graph.toml");
    std::fs::write(&p, s.to_toml_string().unwrap()).unwrap();
    p
}
fn default_spec() -> GraphSpec {
    GraphSpec::from_toml_str(include_str!("../graphs/conversation.default.toml")).unwrap()
}
fn events(p: &Path) -> Vec<wire::RunEvent> {
    JournalReader::read_all(p).unwrap()
}

#[test]
fn invalid_policy_is_rejected_before_execution() {
    for floor in ["INVALID", "l9", ""] {
        let h = Sandbox::new();
        let mut s = default_spec();
        s.policy.evidence.min_level = floor.into();
        let p = save_spec(&h, &s);
        let journal = h.0.join("new.journal");
        let out = h.run(
            &[
                "run",
                p.to_str().unwrap(),
                "--prompt",
                "hi",
                "--offline",
                "--journal",
                journal.to_str().unwrap(),
            ],
            "",
        );
        assert!(!out.status.success());
        assert!(!journal.exists());
    }
    let h = Sandbox::new();
    let mut s = default_spec();
    s.policy.routing.on_quality_fail = Some("reroute".into());
    let p = save_spec(&h, &s);
    assert!(
        !h.run(
            &["run", p.to_str().unwrap(), "--prompt", "hi", "--offline"],
            ""
        )
        .status
        .success()
    );
}
#[test]
fn existing_journal_is_never_overwritten() {
    let h = Sandbox::new();
    let p = h.0.join("existing.journal");
    std::fs::write(&p, b"DO NOT CHANGE").unwrap();
    let out = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "hi",
            "--offline",
            "--journal",
            p.to_str().unwrap(),
        ],
        "",
    );
    assert!(!out.status.success());
    assert_eq!(std::fs::read(p).unwrap(), b"DO NOT CHANGE");
}
#[test]
fn approval_rejection_edit_and_eof_exit_nonzero_without_releasing() {
    for input in ["n\n", "e replacement\n", ""] {
        let h = Sandbox::new();
        let out = h.run(
            &[
                "run",
                "graffy.builtin.conversation.gated",
                "--prompt",
                "ORIGINAL_DRAFT",
                "--offline",
            ],
            input,
        );
        assert!(!out.status.success());
        let output = text(&out);
        assert!(output.contains("ORIGINAL_DRAFT"));
        assert!(output.contains("sha256"));
        assert!(!output.contains("================ reviewed response"));
        let es = events(&h.journals()[0]);
        let decisions: Vec<_> = es
            .iter()
            .filter_map(|e| match &e.event {
                Some(Event::BoundaryEvent(b)) if b.role == "approval_decision" => Some(b),
                _ => None,
            })
            .collect();
        if input.is_empty() {
            assert!(decisions.is_empty());
        } else {
            assert_eq!(decisions[0].content, input.trim_end());
        }
    }
}
#[test]
fn successful_run_has_one_delivered_response_and_replays() {
    let h = Sandbox::new();
    let out = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "hi",
            "--offline",
        ],
        "",
    );
    session(&out);
    let p = &h.journals()[0];
    let es = events(p);
    assert_eq!(
        es.iter()
            .filter(
                |e| matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.visibility=="delivered")
            )
            .count(),
        1
    );
    assert!(h.run(&["replay", p.to_str().unwrap()], "").status.success());
}
#[test]
fn strict_floor_failure_and_late_budget_failure_release_no_answer() {
    let h = Sandbox::new();
    let mut s = default_spec();
    s.policy.evidence.min_level = "L3".into();
    let p = save_spec(&h, &s);
    let out = h.run(
        &["run", p.to_str().unwrap(), "--prompt", "hi", "--offline"],
        "",
    );
    assert!(!out.status.success());
    assert!(!text(&out).contains("================ reviewed response"));
    let h = Sandbox::new();
    let mut s = default_spec();
    s.nodes
        .retain(|n| matches!(n.kind.as_str(), "intake" | "model" | "respond"));
    let mut late = s.nodes.iter().find(|n| n.kind == "model").unwrap().clone();
    late.id = "late".into();
    s.nodes.push(late);
    s.edges = vec![
        EdgeSpec {
            from: "intake".into(),
            to: "draft".into(),
            when: None,
        },
        EdgeSpec {
            from: "draft".into(),
            to: "respond".into(),
            when: None,
        },
        EdgeSpec {
            from: "respond".into(),
            to: "late".into(),
            when: None,
        },
    ];
    s.policy.budget.max_tokens = Some(25);
    let p = save_spec(&h, &s);
    let out = h.run(
        &["run", p.to_str().unwrap(), "--prompt", "hello", "--offline"],
        "",
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("BudgetExhausted"));
    assert!(!text(&out).contains("================"));
    assert!(
        !events(&h.journals()[0])
            .iter()
            .any(|e| matches!(&e.event,Some(Event::BoundaryEvent(b)) if b.visibility=="released"))
    );
}
#[test]
fn unknown_and_corrupt_session_history_fail_loudly() {
    let h = Sandbox::new();
    let out = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "hi",
            "--offline",
            "--session",
            "missing",
        ],
        "",
    );
    assert!(!out.status.success());
    let first = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "hi",
            "--offline",
        ],
        "",
    );
    let ses = session(&first);
    std::fs::write(&h.journals()[0], b"broken").unwrap();
    let out = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "again",
            "--offline",
            "--session",
            &ses,
        ],
        "",
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("complete session history"));
}
#[test]
fn custom_journal_continuation_hydrates_and_explicit_correction_opens_episode() {
    let h = Sandbox::new();
    let custom = h.0.join("elsewhere/custom.journal");
    let first = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "ZEPHYR",
            "--offline",
            "--journal",
            custom.to_str().unwrap(),
        ],
        "",
    );
    let ses = session(&first);
    let next = h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "use the other code",
            "--offline",
            "--session",
            &ses,
            "--correction",
        ],
        "",
    );
    session(&next);
    assert!(text(&next).contains("ZEPHYR"));
    let rated = h.run(&["rate", &ses, "--rater", "fixture"], "\n");
    assert!(rated.status.success());
    assert!(text(&rated).contains("right-censored"));
    let rating = h
        .journals()
        .into_iter()
        .find(|p| p.to_string_lossy().contains("-rating"))
        .unwrap();
    assert!(events(&rating).iter().any(|e|matches!(&e.event,Some(Event::HrdmSampled(s)) if s.repair_cost.is_none() && s.unratable_reasons.contains("right-censored"))));
}
#[test]
fn retry_overflow_and_correction_without_session_are_rejected() {
    let h = Sandbox::new();
    for extra in [vec!["--retry", "4294967295"], vec!["--correction"]] {
        let mut args = vec![
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "hi",
            "--offline",
        ];
        args.extend(extra);
        assert!(!h.run(&args, "").status.success());
    }
}

#[test]
fn reviewed_segmentation_preserves_the_original_and_scores_corrected_units() {
    let h = Sandbox::new();
    let ses = session(&h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "first",
            "--offline",
        ],
        "",
    ));
    for _ in 1..5 {
        session(&h.run(
            &[
                "run",
                "graffy.builtin.conversation",
                "--prompt",
                "next",
                "--offline",
                "--session",
                &ses,
            ],
            "",
        ));
    }
    let preview = h.run(&["rate", &ses], "");
    let printed = text(&preview);
    let hash = printed
        .lines()
        .find_map(|l| l.strip_prefix("snapshot sha256: "))
        .unwrap();
    let correction = serde_json::json!({"proposal_sha256":hash,"note":"independent fixture marked repair","exchanges":[[0,1],[2,3],[4,5],[6,7],[8,9]],"windows":[],"episodes":[{"initiating_turn":2,"closed_at_exchange":3}]});
    let p = h.0.join("correction.json");
    std::fs::write(&p, correction.to_string()).unwrap();
    let out = h.run(
        &["rate", &ses, "--segmentation", p.to_str().unwrap()],
        "\n1\n",
    );
    assert!(out.status.success(), "{}", text(&out));
    let p = h
        .journals()
        .into_iter()
        .find(|p| p.to_string_lossy().contains("-rating"))
        .unwrap();
    let es = events(&p);
    let frozen = es
        .iter()
        .find_map(|e| match &e.event {
            Some(Event::Segmentation(s)) if s.decision == "frozen" => Some(s),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        frozen.proposal_sha256,
        graffy_core::exec::sha256_hex(frozen.proposal.as_bytes())
    );
    let full: serde_json::Value = serde_json::from_str(&frozen.proposal).unwrap();
    assert_eq!(full["turns"].as_array().unwrap().len(), 10);
    assert_eq!(full["windows"], serde_json::json!([[0, 4]]));
    assert!(es.iter().any(|e|matches!(&e.event,Some(Event::Segmentation(s)) if s.decision=="adjusted" && s.correction.contains("independent fixture"))));
    let samples: Vec<_> = es
        .iter()
        .filter_map(|e| match &e.event {
            Some(Event::HrdmSampled(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].repair_cost, Some(1));
}

#[test]
fn invalid_segmentation_choice_and_out_of_window_citation_cannot_silently_score() {
    let h = Sandbox::new();
    let ses = session(&h.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "first",
            "--offline",
        ],
        "",
    ));
    for _ in 1..5 {
        session(&h.run(
            &[
                "run",
                "graffy.builtin.conversation",
                "--prompt",
                "next",
                "--offline",
                "--session",
                &ses,
            ],
            "",
        ));
    }
    for input in ["mistyped\n", "\n2\n99\n"] {
        let out = h.run(&["rate", &ses], input);
        assert!(text(&out).contains("aborted"));
        assert!(
            !h.journals()
                .iter()
                .any(|p| p.to_string_lossy().contains("-rating"))
        );
    }
}

#[test]
fn retries_share_one_total_budget_instead_of_resetting_each_attempt() {
    let baseline = Sandbox::new();
    let mut s = default_spec();
    s.policy.evidence.min_level = "L3".into();
    let p = save_spec(&baseline, &s);
    let out = baseline.run(
        &[
            "run",
            p.to_str().unwrap(),
            "--prompt",
            "hello",
            "--offline",
            "--retry",
            "1",
        ],
        "",
    );
    assert!(!out.status.success());
    let mut paths = baseline.journals();
    paths.sort();
    assert_eq!(paths.len(), 2);
    let totals: Vec<u64> = paths
        .iter()
        .map(|p| {
            events(p)
                .iter()
                .find_map(|e| match &e.event {
                    Some(Event::RunFinished(f)) => {
                        Some(f.total_input_tokens + f.total_output_tokens)
                    }
                    _ => None,
                })
                .unwrap()
        })
        .collect();
    let budget = totals.iter().sum::<u64>() - 1;
    assert!(totals[1] < budget);
    let h = Sandbox::new();
    s.policy.budget.max_tokens = Some(budget);
    let p = save_spec(&h, &s);
    let out = h.run(
        &[
            "run",
            p.to_str().unwrap(),
            "--prompt",
            "hello",
            "--offline",
            "--retry",
            "3",
        ],
        "",
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("BudgetExhausted"));
    assert_eq!(h.journals().len(), 2);
}
