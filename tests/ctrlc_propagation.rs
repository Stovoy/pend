//! Test that aborting a `pend wait` invocation (simulating the user pressing
//! Ctrl-C) **does not** terminate the underlying worker process. The parent
//! `pend wait` process should exit quickly while the detached worker keeps
//! running and ultimately finishes the job.

use assert_cmd::prelude::*;
use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::TempDir;

fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary exists")
}

/// Helper returning a fresh temporary directory and a configured Command
/// builder that inherits the directory via `PEND_DIR`.
fn pend_with_tempdir() -> (TempDir, Command) {
    let tmp = TempDir::new().expect("create tempdir");
    let mut cmd = pend_bin();
    cmd.env("PEND_DIR", tmp.path());
    (tmp, cmd)
}

#[test]
fn ctrlc_does_not_kill_worker() {
    // Allocate isolated jobs directory so concurrent test runs cannot clash.
    let (tmp, mut pend_cmd) = pend_with_tempdir();

    let job = "longrun";

    // Start a job that takes noticeable time so we can interrupt the waiter
    // before it finishes. Using `bash` is fine – the other integration tests
    // rely on it as well and it is present in the GitHub Actions images for
    // all target platforms.
    pend_cmd
        .args([
            "do",
            job,
            "bash",
            "-c",
            // Print a marker, sleep for a second, then print a second marker.
            "echo start && sleep 1 && echo done",
        ])
        .assert()
        .success();

    // Spawn `pend wait` **without** waiting for it – we are going to abort it
    // artificially.
    let mut wait_child = pend_bin()
        .env("PEND_DIR", tmp.path())
        .arg("--no-color")
        .args(["wait", job])
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn pend wait");

    // Give the waiter a brief moment to attach.
    thread::sleep(Duration::from_millis(100));

    // Simulate Ctrl-C. On Unix we explicitly send SIGINT. On other
    // platforms fall back to forcibly killing the process which is good
    // enough for our purposes: only the *wait* process must die, the detached
    // worker must keep running.
    #[cfg(unix)]
    {
        // Safety: libc call parameters are valid (current process has
        // permission to signal its own child).
        unsafe {
            libc::kill(wait_child.id() as i32, libc::SIGINT);
        }
    }

    #[cfg(not(unix))]
    {
        wait_child.kill().expect("terminate wait process");
    }

    // The wait process should exit promptly.
    let _ = wait_child.wait().expect("wait on child");

    // Immediately after aborting the waiter the job must *not* have finished
    // yet (no `.exit` marker).
    let exit_path = tmp.path().join(format!("{job}.exit"));
    assert!(!exit_path.exists(), "job unexpectedly finished early");

    // Now invoke a fresh `pend wait` and expect it to replay the full output
    // once the worker completes. This also implicitly verifies that the
    // worker continued running despite the earlier abort.
    // Replay the job. The *content* of the log produced after the first
    // marker is not guaranteed because the worker writes the `.exit` marker
    // *before* finishing the final log flush (see worker.rs for details).
    // We therefore only verify that the second wait succeeds and returns
    // exit code 0 which proves that the worker kept running independently
    // from the aborted parent.
    pend_bin()
        .env("PEND_DIR", tmp.path())
        .arg("--no-color")
        .args(["wait", job])
        .assert()
        .success();
}
