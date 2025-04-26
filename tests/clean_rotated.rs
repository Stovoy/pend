use assert_cmd::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary under test")
}

fn pend_with_temp() -> (TempDir, Command) {
    let tmp = TempDir::new().expect("tmpdir");
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

#[test]
fn clean_removes_rotated_logs() {
    let (tmp, mut pend) = pend_with_temp();

    // Spawn trivial job and wait for completion so primary artifacts exist.
    pend.args(["do", "rot", "bash", "-c", "echo rot"])
        .assert()
        .success();

    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["wait", "rot"])
        .assert()
        .success();

    // Manually create a rotated log file as produced by log rotation.
    let rotated_path = tmp.path().join("rot.log.1");
    fs::write(&rotated_path, b"old log").unwrap();
    assert!(rotated_path.exists());

    // Cleaning the job should remove *both* the primary and rotated logs.
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["clean", "rot"])
        .assert()
        .success();

    assert!(!rotated_path.exists());
    assert!(fs::read_dir(tmp.path()).unwrap().next().is_none());
}
