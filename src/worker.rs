//! Detached background process launched by `pend do`.
//!
//! A *worker* has exactly one job: run the user command in a sub-process and
//! persist all relevant artifacts (logs, exit code, metadata) in the jobs
//! directory. The code has been extended to optionally enforce a wall-clock
//! timeout and to retry failed attempts a configurable number of times.

use chrono::Utc;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use wait_timeout::ChildExt;

use crate::paths::JobPaths;

/// Metadata written to `<job>.json` once the worker finishes.
#[derive(Serialize)]
struct Meta<'a> {
    job: &'a str,
    cmd: Vec<String>,
    pid: u32,
    started: String,
    ended: String,
    exit_code: i32,
}

/// Spawn a *detached* background worker process responsible for running the
/// actual command and recording artifacts. Front-end helper called by
/// `pend do`.
pub(crate) fn spawn_worker(
    job_name: &str,
    cmd: &[String],
    timeout: Option<u64>,
    retries: Option<u32>,
) -> io::Result<()> {
    let exe_path = std::env::current_exe()?;

    let mut worker_cmd = Command::new(&exe_path);
    worker_cmd.arg("worker").arg(job_name).arg("--");
    worker_cmd.args(cmd);

    // Pass optional runtime configuration via environment variables so the
    // command-line surface of the hidden `worker` sub-command remains
    // stable.
    if let Some(t) = timeout {
        worker_cmd.env("PEND_TIMEOUT", t.to_string());
    }
    if let Some(r) = retries {
        worker_cmd.env("PEND_RETRIES", r.to_string());
    }

    // Detach from controlling terminal so that the worker survives even when
    // the parent exits.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            worker_cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        worker_cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    worker_cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    worker_cmd.spawn()?;
    Ok(())
}

/// Entry point executed by the hidden `worker` sub-command. Never called by
/// end users.
pub(crate) fn run_worker(job_name: &str, cmd: &[String]) -> io::Result<()> {
    // ---------------------------------------------------------------------
    // Resolve paths and obtain an exclusive file lock for the duration of
    // the worker. This guarantees *exactly one* worker per job name.
    // ---------------------------------------------------------------------
    let paths = JobPaths::new(job_name)?;

    use fs2::FileExt;
    let lock_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)?;
    if let Err(err) = lock_file.try_lock_exclusive() {
        if err.kind() == io::ErrorKind::WouldBlock {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("job '{job_name}' is already running"),
            ));
        } else {
            return Err(err);
        }
    }

    // Runtime configuration propagated from the front-end.
    let timeout_secs = std::env::var("PEND_TIMEOUT").ok().and_then(|v| v.parse::<u64>().ok());
    let mut retries_left: u32 = std::env::var("PEND_RETRIES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    // ---------------------------------------------------------------------
    // Helper executing *one* attempt of the user command.
    // ---------------------------------------------------------------------
    fn run_once(
        cmd: &[String],
        paths: &JobPaths,
        timeout_secs: Option<u64>,
        append: bool,
    ) -> io::Result<(i32, chrono::DateTime<Utc>, chrono::DateTime<Utc>, u32)> {
        // Open per-stream artifact files.
        let open_mode = |p: &std::path::Path, append: bool| -> io::Result<File> {
            let mut opts = OpenOptions::new();
            opts.create(true);
            if append {
                opts.append(true);
            } else {
                opts.write(true).truncate(true);
            }
            opts.open(p)
        };

        let out_file = open_mode(&paths.out, append)?;
        let err_file = open_mode(&paths.err, append)?;

        // Combined log file and rotation support.
        let mut log_file = open_mode(&paths.log, append)?;
        if append {
            let _ = writeln!(log_file, "\n-- retry --\n");
        }

        let max_log_size = std::env::var("PEND_MAX_LOG_SIZE")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());

        let log_path_clone = paths.log.clone();
        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        let writer_handle = std::thread::spawn(move || -> io::Result<()> {
            let mut current_len = log_file.metadata().map(|m| m.len()).unwrap_or(0);
            while let Ok(chunk) = rx.recv() {
                if let Some(limit) = max_log_size {
                    if current_len + chunk.len() as u64 > limit {
                        let rotated = log_path_clone.with_file_name(format!(
                            "{}.1",
                            log_path_clone.file_name().unwrap().to_string_lossy()
                        ));
                        let _ = fs::rename(&log_path_clone, &rotated);
                        log_file = OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(&log_path_clone)?;
                        current_len = 0;
                    }
                }
                log_file.write_all(&chunk)?;
                current_len += chunk.len() as u64;
            }
            Ok(())
        });

        // Spawn child process.
        let started = Utc::now();
        let mut child = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout_pipe = child.stdout.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "failed to capture stdout")
        })?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "failed to capture stderr")
        })?;

        // Reader helper feeding per-stream artifacts *and* combined log.
        fn spawn_reader<R: Read + Send + 'static>(
            reader: R,
            mut dest: File,
            tx: mpsc::Sender<Vec<u8>>,
        ) -> std::thread::JoinHandle<io::Result<()>> {
            std::thread::spawn(move || {
                let mut buf = std::io::BufReader::new(reader);
                let mut chunk = [0u8; 8192];
                loop {
                    let n = match buf.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) => return Err(e),
                    };
                    dest.write_all(&chunk[..n])?;
                    let _ = tx.send(chunk[..n].to_vec());
                }
                Ok(())
            })
        }

        let r1 = spawn_reader(stdout_pipe, out_file, tx.clone());
        let r2 = spawn_reader(stderr_pipe, err_file, tx);

        // Wait with optional timeout.
        let status = if let Some(secs) = timeout_secs {
            match child.wait_timeout(Duration::from_secs(secs))? {
                Some(s) => s,
                None => {
                    let _ = child.kill();
                    child.wait()?
                }
            }
        } else {
            child.wait()?
        };

        // Join helper threads.
        for h in [r1, r2] {
            match h.join() {
                Ok(res) => res?,
                Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "reader thread panicked")),
            }
        }

        match writer_handle.join() {
            Ok(res) => res?,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "writer thread panicked")),
        }

        let ended = Utc::now();

        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;

        let mut exit_code = 1;
        #[cfg(unix)]
        let mut terminated_signal: Option<i32> = None;

        match status.code() {
            Some(c) => exit_code = c,
            None => {
                #[cfg(unix)]
                if let Some(sig) = status.signal() {
                    terminated_signal = Some(sig);
                    exit_code = 128 + sig;
                }
            }
        }

        #[cfg(unix)]
        if let Some(sig) = terminated_signal {
            let _ = fs::write(&paths.signal, format!("{}\n", sig));
        }

        Ok((exit_code, started, ended, child.id()))
    }

    // ------------------------------------------------------------------
    // Retry loop.
    // ------------------------------------------------------------------

    let (mut final_exit_code, first_started, mut last_ended, mut final_pid) =
        run_once(cmd, &paths, timeout_secs, false)?;

    let append = true; // subsequent attempts should append to existing log files

    while final_exit_code != 0 && retries_left > 0 {
        retries_left -= 1;

        let (code, _started, ended, pid) = run_once(cmd, &paths, timeout_secs, append)?;

        // The first_started timestamp is intentionally preserved from the very
        // first attempt, but we keep updating the other fields so that the
        // metadata reflects the details from the last attempt.
        last_ended = ended;
        final_pid = pid;
        final_exit_code = code;
    }

    // ------------------------------------------------------------------
    // Persist exit code and metadata.
    // ------------------------------------------------------------------
    fs::write(&paths.exit, format!("{}\n", final_exit_code))?;

    let meta = Meta {
        job: job_name,
        cmd: cmd.to_vec(),
        pid: final_pid,
        started: first_started.to_rfc3339(),
        ended: last_ended.to_rfc3339(),
        exit_code: final_exit_code,
    };
    let json = serde_json::to_vec_pretty(&meta)?;
    fs::write(&paths.meta, json)?;

    // All artifacts persisted – drop the advisory lock and delete the file so
    // the presence of a lingering `.lock` does not confuse future commands.
    drop(lock_file); // explicit – ensures the exclusive lock is released first
    let _ = fs::remove_file(&paths.lock);

    Ok(())
}
