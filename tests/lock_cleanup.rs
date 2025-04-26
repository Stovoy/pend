use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary")
}

fn pend_with_temp() -> (TempDir, Command) {
    let tmp = TempDir::new().expect("tmpdir");
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

#[test]
fn lock_removed_after_job_finishes() {
    let (tmp, mut pend) = pend_with_temp();

    pend.args(["do", "cleanlock", "bash", "-c", "echo hi"])
        .assert()
        .success();

    // Wait for job completion.
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["wait", "cleanlock"])
        .assert()
        .success();

    // `.lock` should have been removed by the worker.
    let lock_path = tmp.path().join("cleanlock.lock");
    assert!(!lock_path.exists(), "lock file still present after completion");

    // Optionally, running another job with the same name would still be
    // blocked by existing artifacts. The important bit for this test is that
    // the *lock file* itself no longer exists.
}
