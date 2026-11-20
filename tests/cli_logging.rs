//! Plain commands are quiet by default (issue #34): INFO log lines appear only
//! when asked for with `-v` or `RUST_LOG`, warnings stay visible, and every
//! diagnostic goes to stderr so stdout is the command's own output.
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
};

/// The INFO line `open_store` logs when it seeds the built-in graphs into a
/// fresh home — the line issue #34 saw in `graffy doctor`.
const SEED_LOG: &str = "registered/updated built-in graphs";
/// The INFO line the executor logs for every node.
const NODE_LOG: &str = "node running";
/// The WARN line a `GRAFFY_MODEL_*` value without a `provider:model` shape gets.
const MALFORMED_BINDING: &str = "ignoring malformed binding";

struct Sandbox(PathBuf);
impl Sandbox {
    /// A fresh home, so the first store open always seeds (and logs).
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "graffy-cli-logging-{}",
            graffy_core::id::RunId::generate()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_graffy"));
        c.env("GRAFFY_HOME", &self.0)
            .env_remove("GRAFFY_DATA_DIR")
            // Only what a test passes in decides the filter and the bindings.
            .env_remove("RUST_LOG");
        for (key, _) in std::env::vars() {
            if key.starts_with("GRAFFY_MODEL_") {
                c.env_remove(key);
            }
        }
        c.envs(env.iter().copied())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn assert_no_info(o: &Output) {
    for (stream, text) in [("stdout", stdout(o)), ("stderr", stderr(o))] {
        assert!(
            !text.contains("INFO") && !text.contains(SEED_LOG) && !text.contains(NODE_LOG),
            "an INFO log line reached {stream} by default:\n{text}"
        );
    }
}

#[test]
fn doctor_prints_no_info_logs_by_default() {
    let home = Sandbox::new();
    let out = home.run(&["doctor"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    // The report itself is intact, and the seeding really happened.
    assert!(stdout(&out).contains("graphs "), "{}", stdout(&out));
    assert_no_info(&out);
    assert_eq!(stderr(&out), "", "doctor has nothing to warn about here");
}

#[test]
fn mcp_add_prints_no_info_logs_by_default() {
    let home = Sandbox::new();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/crates/graffy-mcp/tests/fixture/mini_server.py"
    );
    let out = home.run(
        &[
            "mcp",
            "add",
            "fixture",
            "--stdio",
            &format!("python3 {fixture}"),
            "--skip-interview",
        ],
        &[],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("registered server 'fixture'"),
        "{}",
        stdout(&out)
    );
    assert_no_info(&out);
}

#[test]
fn run_shows_its_steps_without_log_lines() {
    let home = Sandbox::new();
    let out = home.run(
        &[
            "run",
            "graffy.builtin.conversation",
            "--prompt",
            "hi",
            "--offline",
        ],
        &[],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_no_info(&out);
    // Progress still reaches the human: one line per node, on stderr.
    for node in ["intake", "ground", "draft", "verify", "respond"] {
        assert!(
            stderr(&out).contains(&format!("· step: {node}")),
            "missing step line for {node}:\n{}",
            stderr(&out)
        );
    }
    assert!(
        !stdout(&out).contains("· step:"),
        "step progress belongs on stderr:\n{}",
        stdout(&out)
    );
}

#[test]
fn verbose_flag_and_rust_log_bring_info_back_on_stderr() {
    for (args, env) in [
        (&["-v", "doctor"][..], &[][..]),
        (&["-vv", "doctor"][..], &[][..]),
        (&["doctor"][..], &[("RUST_LOG", "info")][..]),
    ] {
        let home = Sandbox::new();
        let out = home.run(args, env);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(
            stderr(&out).contains(SEED_LOG),
            "{args:?} {env:?} should show the INFO line on stderr:\n{}",
            stderr(&out)
        );
        assert!(
            !stdout(&out).contains(SEED_LOG),
            "{args:?} {env:?} logged onto stdout:\n{}",
            stdout(&out)
        );
    }
}

#[test]
fn rust_log_can_still_silence_everything() {
    let home = Sandbox::new();
    let out = home.run(
        &["doctor"],
        &[("RUST_LOG", "off"), ("GRAFFY_MODEL_FAST", "no-colon-here")],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "", "RUST_LOG=off overrides the default");
}

#[test]
fn warnings_stay_visible_by_default_and_keep_off_stdout() {
    let home = Sandbox::new();
    let out = home.run(&["doctor"], &[("GRAFFY_MODEL_FAST", "no-colon-here")]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(MALFORMED_BINDING),
        "the warning must not be silenced:\n{}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains(MALFORMED_BINDING),
        "the warning leaked into the report:\n{}",
        stdout(&out)
    );
    assert_no_info(&out);
}
