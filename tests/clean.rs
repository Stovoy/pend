use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary")
}

fn pend_with_temp() -> (TempDir, Command) {
    let tmp = TempDir::new().expect("tmp");
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

#[test]
fn clean_specific_job() {
    let (tmp, mut pend) = pend_with_temp();

    // create artifacts via quick command
    pend.args(["do", "jobx", "bash", "-c", "echo hi"]).assert().success();
    // wait to finish
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["wait", "jobx"])
        .assert()
        .success();

    let log_path = tmp.path().join("jobx.log");
    assert!(log_path.exists());

    // run clean for jobx
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["clean", "jobx"])
        .assert()
        .success();

    assert!(!log_path.exists());
}

#[test]
fn clean_all() {
    let (tmp, mut pend) = pend_with_temp();

    pend.args(["do", "one", "bash", "-c", "echo 1"]).assert().success();
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["wait", "one"])
        .assert()
        .success();

    pend_bin()
        .env("PEND_DIR", tmp.path())
        .args(["clean", "--all"])
        .assert()
        .success();

    assert!(fs::read_dir(tmp.path()).unwrap().next().is_none());
}
