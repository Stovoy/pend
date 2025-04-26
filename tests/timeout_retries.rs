//! Integration tests for the new `--timeout` and `--retries` features that were
//! recently added to `pend do`.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// Return a `Command` configured to execute the compiled `pend` binary.
fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary exists")
}

// Helper allocating an isolated temporary jobs directory and returning it
// together with a `Command` builder that already has `PEND_DIR` set.
fn pend_with_tmpdir() -> (TempDir, Command) {
    let tmp = TempDir::new().expect("create temp dir");
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

/// Verify that the `--timeout` flag terminates long-running commands and that
/// `pend wait` exits with a non-zero status in that case.
#[test]
fn timeout_enforced() {
    // Skip test gracefully if no python interpreter is available.
    let python = ["python3", "python"]
        .iter()
        .find(|prog| Command::new(prog).arg("--version").output().is_ok())
        .cloned();

    let Some(python) = python else {
        eprintln!(
            "warning: skipping timeout_enforced test – no python interpreter found in PATH"
        );
        return;
    };

    let (tmp, mut pend_cmd) = pend_with_tmpdir();

    // Launch background job that sleeps for 5 seconds but impose a 1-second
    // timeout so the worker must kill the process.
    pend_cmd
        .args([
            "do",
            "timeoutjob",
            "--timeout",
            "1",
            python,
            "-c",
            "import time; time.sleep(5)",
        ])
        .assert()
        .success();

    // `pend wait` should finish quickly (well below the 5 s sleep) and report
    // a failure exit status.
    let start = Instant::now();
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["--no-color", "wait", "timeoutjob"])
        .assert()
        .failure();
    let elapsed = start.elapsed();

    // Allow some slack for CI variance but the whole wait should still be
    // noticeably shorter than the original 5 s runtime.
    assert!(
        elapsed < Duration::from_secs(4),
        "expected timeout to abort the job early (elapsed = {elapsed:?})"
    );
}

/// Verify that the `--retries` flag re-runs a failing command and ultimately
/// reports success when one of the attempts exits with status 0.
#[test]
fn retries_eventual_success() {
    // As above, locate a Python interpreter.
    let python = ["python3", "python"]
        .iter()
        .find(|prog| Command::new(prog).arg("--version").output().is_ok())
        .cloned();

    let Some(python) = python else {
        eprintln!(
            "warning: skipping retries_eventual_success test – no python interpreter found in PATH"
        );
        return;
    };

    let script_dir = TempDir::new().expect("create script dir");
    let sentinel_path = script_dir.path().join("sentinel.txt");

    // Write helper script that fails the first time and succeeds once the
    // sentinel file exists.
    let helper_script = script_dir.path().join("flaky.py");
    std::fs::write(
        &helper_script,
        format!(
            r#"import os, sys, pathlib
sentinel = pathlib.Path(r"{}")
if sentinel.exists():
    print("second run – success")
    sys.exit(0)
else:
    sentinel.write_text("created")
    print("first run – failing")
    sys.exit(3)
"#,
            sentinel_path.display()
        ),
    )
    .expect("write helper script");

    let (tmp, mut pend_cmd) = pend_with_tmpdir();

    // Start background job with exactly one retry (`--retries 1`). The first
    // attempt will exit 3, the second should exit 0.
    pend_cmd
        .args([
            "do",
            "flakyjob",
            "--retries",
            "1",
            python,
            helper_script.to_str().unwrap(),
        ])
        .assert()
        .success();

    // `pend wait` must replay *both* runs (we look for the success marker)
    // and exit with status 0.
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["--no-color", "wait", "flakyjob"])
        .assert()
        .success()
        .stdout(predicate::str::contains("second run – success"));
}
