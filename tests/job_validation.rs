use assert_cmd::prelude::*;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

// Helper to invoke compiled `pend` binary with a temporary jobs directory.
fn pend_with_tmp() -> (TempDir, assert_cmd::Command) {
    let tmp = TempDir::new().expect("create tempdir");
    let mut cmd = assert_cmd::Command::cargo_bin("pend").expect("binary exists");
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

#[test]
fn rejects_leading_dot() {
    let (_tmp, mut cmd) = pend_with_tmp();
    cmd.args(["do", ".hidden", "echo", "oops"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("job name"));
}

#[test]
fn rejects_repeated_dots() {
    let (_tmp, mut cmd) = pend_with_tmp();
    cmd.args(["do", "name..oops", "echo", "oops"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("job name"));
}

#[test]
fn rejects_too_long() {
    let (_tmp, mut cmd) = pend_with_tmp();
    let long_name = "x".repeat(101);
    cmd.args(["do", &long_name, "echo", "oops"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("job name"));
}
