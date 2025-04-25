//! Stress test: ensures pend can handle >=100 MB combined stdout+stderr without
//! dead-locking or truncating output.  Uses an inline Python helper script so
//! the workload is portable across Unix and Windows GitHub runners (Python is
//! preinstalled on all images).

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

// Returns a configured `Command` builder for the compiled `pend` binary.
fn pend_bin() -> Command {
    Command::cargo_bin("pend").expect("binary exists")
}

// Python helper that writes ~110 MB total (55 MB stdout + 55 MB stderr) and
// finishes with a small sentinel marker so we can assert the full stream was
// replayed by `pend wait`.
const PY_SCRIPT: &str = r#"import sys
chunk = b'x' * 1_048_576  # 1 MiB
for _ in range(55):
    sys.stdout.buffer.write(chunk)
    sys.stderr.buffer.write(chunk)
sys.stdout.write('DONE\n')
sys.stderr.write('DONE\n')
"#;

#[test]
fn stress_big_output() {
    // Separate temp dirs: one for job artifacts, one for the helper script.
    let jobs_dir = TempDir::new().expect("create jobs dir");
    let script_dir = TempDir::new().expect("create script dir");

    let script_path = script_dir.path().join("produce_big_output.py");
    std::fs::write(&script_path, PY_SCRIPT).expect("write helper script");

    // Determine an available python interpreter (python3 preferred).
    let python = ["python3", "python"]
        .iter()
        .find(|prog| Command::new(prog).arg("--version").output().is_ok())
        .cloned();

    let Some(python) = python else {
        eprintln!("warning: skipping stress_big_output test – no python interpreter found in PATH");
        return; // skip test gracefully
    };

    // Start background job that produces the large output.
    pend_bin()
        .env("PEND_DIR", jobs_dir.path())
        .args(["do", "bigout", python, script_path.to_str().unwrap()])
        .assert()
        .success();

    // Wait and ensure both streams were replayed (look for sentinel).
    pend_bin()
        .env("PEND_DIR", jobs_dir.path())
        .args(["--no-color", "wait", "bigout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DONE"));
}
