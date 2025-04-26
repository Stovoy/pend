use assert_cmd::prelude::*;
use predicates::str::contains;
use std::process::Command;
use tempfile::TempDir;

fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary")
}

#[test]
fn wait_on_non_existing_job_exits_fast() {
    let tmp = TempDir::new().expect("tmp");
    // Attempt to wait for a job that never existed.
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path())
        .args(["wait", "ghost"])
        .assert()
        .failure()
        .stderr(contains("not found"));
}
