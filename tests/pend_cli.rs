use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::env;
use std::process::Command;
use tempfile::TempDir;

fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary exists")
}

/// Helper to create a new temporary jobs directory and return it *plus* a
/// configured Command builder pointing at `pend` with that environment set.
fn pend_with_tempdir() -> (TempDir, Command) {
    let tmp = TempDir::new().expect("create tempdir");
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

#[test]
fn simple_success_flow() {
    let (tmp, mut cmd) = pend_with_tempdir();

    // `pend do okjob <pend> --version` runs quickly and exits 0.
    let pend_path = assert_cmd::cargo::cargo_bin("pend");

    cmd.args(["do", "okjob", pend_path.to_str().unwrap(), "--version"])
        .assert()
        .success();

    // Wait must reproduce version string and exit 0.
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["wait", "okjob"])
        .assert()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")))
        .success();
}

#[test]
fn propagates_failure_exit_code() {
    let (tmp, mut cmd) = pend_with_tempdir();

    let pend_path = assert_cmd::cargo::cargo_bin("pend");

    cmd.args(["do", "failjob", pend_path.to_str().unwrap(), "--invalid-flag"])
        .assert()
        .success();

    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["wait", "failjob"])
        .assert()
        .code(2)
        .failure();
}

#[test]
fn dir_flag_overrides_env() {
    // Provide both PEND_DIR env and --dir flag; flag should take precedence.
    let env_dir = TempDir::new().unwrap();
    let flag_dir = TempDir::new().unwrap();

    let pend_path = assert_cmd::cargo::cargo_bin("pend");

    // do
    Command::new(&pend_path)
        .env("PEND_DIR", env_dir.path())
        .args([
            "--dir",
            flag_dir.path().to_str().unwrap(),
            "do",
            "flagjob",
            "bash",
            "-c",
            "echo fromflag",
        ])
        .assert()
        .success();

    // wait – rely on PEND_DIR env to *not* find job; should only exist in flag_dir.
    Command::new(&pend_path)
        .env("PEND_DIR", env_dir.path())
        .args(["--dir", flag_dir.path().to_str().unwrap(), "wait", "flagjob"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fromflag"));

    // Verify artifacts exist in flag_dir, not env_dir.
    assert!(flag_dir.path().join("flagjob.out").exists());
    assert!(!env_dir.path().join("flagjob.out").exists());
}
