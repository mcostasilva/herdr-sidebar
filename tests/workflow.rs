//! Black-box tests: real plugin processes, isolated state, no running agents.

use serde_json::{Value, json};
use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        let root = fixture.dir.path();
        fs::create_dir(root.join("state")).unwrap();
        fs::create_dir(root.join("project with spaces")).unwrap();
        for name in ["herdr", "opencode"] {
            fs::write(root.join(name), include_str!("support/fake-cli.sh")).unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let source = json!({"pane_id":"w1:p1","terminal_id":"term_source","agent":"opencode","agent_session":{"agent":"opencode","kind":"id","value":"ses_parent"}});
        let side = json!({"pane_id":"w1:p2","terminal_id":"term_side","agent":"opencode","agent_session":{"agent":"opencode","kind":"id","value":"ses_fork"}});
        fixture.write("source.json", &source);
        fixture.write("side.json", &side);
        fixture.write(
            "starting.json",
            &json!({"pane_id":"w1:p2","terminal_id":"term_side"}),
        );
        fixture.write(
            "opened.json",
            &json!({"result":{"plugin_pane":{"pane":side}}}),
        );
        fixture.write("parent.json", &json!({"data":{"id":"ses_parent","location":{"directory":root.join("project with spaces")},"agent":"build","extra":"ignored"}}));
        fixture.write("fork.json", &json!({"data":{"id":"ses_fork","location":{"directory":root.join("project with spaces")},"fork":{"sessionID":"ses_parent"}}}));
        fixture.write("process.json", &json!({"result":{"process_info":{"foreground_processes":[{"argv":[root.join("opencode"),"--server","http://localhost:4096"],"cwd":root}]}}}));
        fixture
    }

    fn write(&self, path: &str, value: &Value) {
        fs::write(
            self.dir.path().join(path),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn marker(&self, path: &str) {
        fs::write(self.dir.path().join(path), "").unwrap();
    }
    fn remove(&self, path: &str) {
        fs::remove_file(self.dir.path().join(path)).unwrap();
    }
    fn count(&self, path: &str) -> usize {
        fs::read_to_string(self.dir.path().join(path))
            .unwrap_or_default()
            .lines()
            .count()
    }
    fn command(&self, args: &[&str]) -> Command {
        let root = self.dir.path();
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-sidebar"));
        command
            .args(args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", "w1:p1")
            .env("HERDR_SOCKET_PATH", root.join("herdr.sock"))
            .env("HERDR_PLUGIN_STATE_DIR", root.join("state"))
            .env("HERDR_BIN_PATH", root.join("herdr"))
            .env("SIDEBAR_FIXTURE", root)
            .env("HERDR_SIDEBAR_TIMEOUT_MS", "800")
            .env_remove("HERDR_PLUGIN_CONTEXT_JSON");
        command
    }
    fn success(&self, args: &[&str]) -> Value {
        let output = self.command(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn failure(&self, args: &[&str], message: &str) -> Output {
        let output = self.command(args).output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

#[test]
fn creates_then_focuses_from_source_and_side_without_new_forks() {
    let f = Fixture::new();
    assert_eq!(f.success(&["open"])["action"], "created");
    assert_eq!(f.success(&["open"])["action"], "focused");
    assert_eq!(f.success(&["open", "--pane", "w1:p2"])["action"], "focused");
    assert_eq!(f.count("forks"), 1);
    assert_eq!(f.count("opens"), 1);
    let args = fs::read_to_string(f.dir.path().join("launch-args")).unwrap();
    assert!(args.contains("--direction\nright\n--target-pane\nw1:p1\n"));
    assert!(args.contains(f.dir.path().join("project with spaces").to_str().unwrap()));
    assert!(args.contains("--plugin\nherdr-sidebar\n"));
    assert!(args.contains("HERDR_SIDEBAR_SERVER=http://localhost:4096"));
    assert!(args.contains("HERDR_SIDEBAR_SESSION=ses_fork"));
}

#[test]
fn concurrent_invocations_create_exactly_one_fork_and_pane() {
    let f = Fixture::new();
    let a = f.command(&["open"]).spawn().unwrap();
    let b = f.command(&["open"]).spawn().unwrap();
    for child in [a, b] {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(f.count("forks"), 1);
    assert_eq!(f.count("opens"), 1);
}

#[test]
fn closed_side_gets_a_fresh_fork() {
    let f = Fixture::new();
    f.success(&["open"]);
    f.remove("side");
    assert_eq!(f.success(&["open"])["action"], "created");
    assert_eq!(f.count("forks"), 2);
}

#[test]
fn lost_pane_response_is_reconciled_without_duplicate_launch() {
    let f = Fixture::new();
    f.marker("lose-open-response");
    f.failure(&["open"], "fork ses_fork is saved");
    f.remove("lose-open-response");
    f.success(&["open"]);
    assert_eq!(f.count("opens"), 1);
    assert_eq!(f.count("forks"), 1);
}

#[test]
fn lost_fork_response_requires_validated_explicit_recovery() {
    let f = Fixture::new();
    f.marker("lose-fork-response");
    f.failure(&["open"], "fork outcome could not be confirmed");
    f.failure(&["open"], "previous fork outcome is unknown");
    assert_eq!(f.count("forks"), 1);
    assert_eq!(f.count("opens"), 0);
    f.failure(&["open", "--recover-session", "ses_parent"], "not a fork");
    f.remove("lose-fork-response");
    f.success(&["open", "--recover-session", "ses_fork"]);
    assert_eq!(f.count("forks"), 1);
}

#[test]
fn rejected_launch_reuses_saved_fork_after_explicit_retry() {
    let f = Fixture::new();
    f.marker("reject-open");
    f.failure(&["open"], "fork ses_fork is saved");
    f.remove("reject-open");
    f.failure(&["open"], "previous pane launch outcome is unknown");
    f.success(&["open", "--retry-launch"]);
    assert_eq!(f.count("forks"), 1);
}

#[test]
fn startup_is_verified_and_retried_without_duplicate_panes() {
    let f = Fixture::new();
    f.marker("not-ready");
    f.failure(&["open"], "did not attach");
    f.remove("not-ready");
    f.success(&["open"]);
    assert_eq!(f.count("opens"), 1);
    assert_eq!(f.count("forks"), 1);
}

#[test]
fn malformed_read_response_does_not_fork_or_change_state() {
    let f = Fixture::new();
    f.marker("malformed");
    f.failure(&["open"], "invalid JSON");
    assert_eq!(f.count("forks"), 0);
    assert!(!f.dir.path().join("state/state.json").exists());
}

#[test]
fn subprocess_and_lock_waits_are_bounded() {
    let f = Fixture::new();
    f.marker("hang");
    let start = Instant::now();
    f.failure(&["open"], "timed out");
    assert!(start.elapsed() < Duration::from_secs(5));
    f.remove("hang");
    let lock = File::options()
        .read(true)
        .write(true)
        .open(f.dir.path().join("state/state.lock"))
        .unwrap();
    lock.lock().unwrap();
    let start = Instant::now();
    f.failure(&["open"], "another sidebar action");
    assert!(start.elapsed() < Duration::from_secs(5));
    drop(lock);
    f.success(&["open"]);
}

#[test]
fn unwritable_state_prevents_remote_side_effects() {
    let f = Fixture::new();
    // A directory at the temporary-file path fails even when tests run as root.
    fs::create_dir(f.dir.path().join("state/state.tmp")).unwrap();
    let result = f.command(&["open"]).output().unwrap();
    assert!(!result.status.success());
    assert_eq!(f.count("forks"), 0);
    assert_eq!(f.count("opens"), 0);
}

#[test]
fn invocation_snapshot_wins_over_inherited_caller_id() {
    let f = Fixture::new();
    let output = f
        .command(&["open"])
        .env("HERDR_PANE_ID", "w1:p99")
        .env(
            "HERDR_PLUGIN_CONTEXT_JSON",
            r#"{"focused_pane_id":"w1:p1"}"#,
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn help_version_and_usage_errors_work_outside_herdr() {
    for args in [&["--help"][..], &["open", "--help"], &["--version"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_herdr-sidebar"))
            .args(args)
            .env_remove("HERDR_ENV")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_herdr-sidebar"))
        .arg("nonsense")
        .env_remove("HERDR_ENV")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn attach_executes_exact_session_and_server_arguments() {
    let f = Fixture::new();
    let binary = f.dir.path().join("capture");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$SIDEBAR_FIXTURE/attached\"\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let output = f
        .command(&["attach"])
        .env("HERDR_SIDEBAR_BIN", &binary)
        .env("HERDR_SIDEBAR_SESSION", "ses_fork")
        .env("HERDR_SIDEBAR_SERVER", "http://localhost:4096")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(f.dir.path().join("attached")).unwrap(),
        "--server\nhttp://localhost:4096\n--session\nses_fork\n"
    );
}

#[test]
fn corrupt_state_is_preserved() {
    let f = Fixture::new();
    let path = f.dir.path().join("state/state.json");
    fs::write(&path, "broken").unwrap();
    f.failure(&["open"], "invalid sidebar state");
    assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    assert_eq!(f.count("forks"), 0);
}

#[test]
fn empty_session_is_rejected_before_recording_a_fork_intent() {
    let f = Fixture::new();
    f.marker("empty-session");
    f.failure(&["open"], "conversation is empty");
    assert_eq!(f.count("forks"), 0);
    assert!(!f.dir.path().join("state/state.json").exists());
    f.remove("empty-session");
    f.success(&["open"]);
}

#[test]
fn failed_fork_does_not_create_a_pane_and_can_be_explicitly_retried() {
    let f = Fixture::new();
    f.marker("fail-fork");
    f.failure(&["open"], "fork outcome could not be confirmed");
    f.failure(&["open"], "previous fork outcome is unknown");
    assert_eq!(f.count("opens"), 0);
    assert_eq!(f.count("forks"), 1);
    f.remove("fail-fork");
    f.success(&["open", "--retry-fork"]);
    assert_eq!(f.count("forks"), 2);
}
