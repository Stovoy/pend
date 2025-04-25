use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::paths::JobPaths;

/// Lightweight metadata structure serialised to JSON once a job finishes.
#[derive(Serialize)]
struct Meta<'a> {
    job: &'a str,
    cmd: Vec<String>,
    pid: u32,
    started: String,
    ended: String,
    exit_code: i32,
}

/// Spawn a detached **worker** process which, in turn, executes the actual
/// command and records artifacts. This helper is invoked by [`crate::job::do_job`].
pub(crate) fn spawn_worker(job_name: &str, cmd: &[String]) -> io::Result<()> {
    let exe_path = env::current_exe()?;
    let mut worker_cmd = Command::new(&exe_path);
    worker_cmd.arg("worker").arg(job_name).arg("--");
    worker_cmd.args(cmd);

    // Detach: we do *not* inherit stdin/stdout/stderr to avoid mixing logs.
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

    worker_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    worker_cmd.spawn()?;
    Ok(())
}

/// Internal function executed by the *worker* sub-command.
/// Entry point used by the hidden `worker` CLI subcommand.
pub(crate) fn run_worker(job_name: &str, cmd: &[String]) -> io::Result<()> {
    let paths = JobPaths::new(job_name)?;

    // ------------------------------------------------------------------
    // Obtain the same advisory file lock that `pend do` used for the brief
    // initialisation window. Holding the lock for the entire lifetime of
    // the worker process guarantees that *no* second job with the same
    // name can start while this one is still executing, even after the
    // parent process has exited and released its short-lived lock.
    // ------------------------------------------------------------------

    use fs2::FileExt;
    use std::fs::OpenOptions;

    let lock_file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&paths.lock)?;

    if let Err(err) = lock_file.try_lock_exclusive() {
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("job '{job_name}' is already running"),
            ));
        } else {
            return Err(err);
        }
    }

    // We'll capture stdout/stderr via pipes so that we can merge them while
    // still writing dedicated .out / .err files.
    let started = chrono::Utc::now();

    let mut child_proc = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Capture stdout / stderr pipes from the child process.
    let stdout_pipe = child_proc
        .stdout
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to capture stdout"))?;
    let stderr_pipe = child_proc
        .stderr
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to capture stderr"))?;

    // Writers for individual streams plus the combined log.
    let out_file = File::create(&paths.out)?;
    let err_file = File::create(&paths.err)?;
    // ------------------------------------------------------------------
    // New: use a single dedicated writer task for the combined log file to
    // avoid contended locking between the two stream reader threads.
    // ------------------------------------------------------------------

    use std::sync::mpsc;

    // Channel capacity tuned to roughly one decent chunk per stream – we do
    // not need a huge buffer because the writer keeps up easily.
    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    // Dedicated writer thread which owns the combined log file handle.
    // Read max log size from environment once.
    let max_log_size = std::env::var("PEND_MAX_LOG_SIZE").ok().and_then(|v| v.parse::<u64>().ok());

    let log_handle = {
        let log_path_clone = paths.log.clone();
        std::thread::spawn(move || -> io::Result<()> {
            let mut log_file = File::create(&log_path_clone)?;
            let mut current_len: u64 = 0;
            if let Ok(meta) = log_file.metadata() {
                current_len = meta.len();
            }

            while let Ok(chunk) = rx.recv() {
                if let Some(limit) = max_log_size {
                    if current_len + chunk.len() as u64 > limit {
                        // Rotate: rename current file to .log.1, ignoring errors.
                        let rotated = log_path_clone.with_file_name(format!(
                            "{}.1",
                            log_path_clone
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .to_string()
                        ));
                        let _ = std::fs::rename(&log_path_clone, &rotated);
                        // Start new file.
                        log_file = File::create(&log_path_clone)?;
                        current_len = 0;
                    }
                }

                log_file.write_all(&chunk)?;
                current_len += chunk.len() as u64;
            }
            Ok(())
        })
    };

    // Helper spawning one reader thread per pipe that forwards bytes to the
    // per-stream artefact file and to the shared channel.
    fn spawn_reader<R: Read + Send + 'static>(
        reader: R,
        mut dest_file: File,
        tx: mpsc::Sender<Vec<u8>>,
    ) -> std::thread::JoinHandle<io::Result<()>> {
        std::thread::spawn(move || {
            let mut buf_reader = std::io::BufReader::new(reader);
            let mut chunk = [0u8; 8192];
            loop {
                let n = match buf_reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => return Err(e),
                };

                dest_file.write_all(&chunk[..n])?;
                // Ignore send errors – means the writer thread already shut down.
                let _ = tx.send(chunk[..n].to_vec());
            }
            Ok(())
        })
    }

    let stdout_handle = spawn_reader(stdout_pipe, out_file, tx.clone());
    let stderr_handle = spawn_reader(stderr_pipe, err_file, tx);

    // Wait for the child process to exit *and* for the reader threads to
    // finish flushing their respective buffers.
    let status = child_proc.wait()?;

    // ------------------------------------------------------------------
    // Determine a portable numeric exit code.
    //
    // On Unix-like systems a process that terminates due to a signal does
    // not have a conventional exit status.  The idiomatic convention used
    // by many tools (bash, coreutils, git, etc.) is to report *128 + signal*.
    // Capturing this information allows the parent `pend wait` invocation to
    // faithfully propagate failure causes such as SIGKILL or SIGTERM.
    //
    // On non-Unix platforms we fall back to the existing behaviour.
    // ------------------------------------------------------------------

    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    let mut exit_code = 1;

    #[cfg(unix)]
    let mut terminated_signal: Option<i32> = None;

    match status.code() {
        Some(code) => exit_code = code,
        None => {
            #[cfg(unix)]
            {
                if let Some(sig) = status.signal() {
                    terminated_signal = Some(sig);
                    exit_code = 128 + sig;
                }
            }
        }
    }

    // Write exit code and, if available, signal file early.
    fs::write(&paths.exit, format!("{}\n", exit_code))?;

    #[cfg(unix)]
    if let Some(sig) = terminated_signal {
        let _ = fs::write(&paths.signal, format!("{}\n", sig));
    }

    let join_and_check = |handle: std::thread::JoinHandle<io::Result<()>>| -> io::Result<()> {
        match handle.join() {
            Err(join_err) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("log thread panicked: {:?}", join_err),
            )),
            Ok(res) => res,
        }
    };

    join_and_check(stdout_handle)?;
    join_and_check(stderr_handle)?;

    // Wait for writer.
    join_and_check(log_handle)?;

    let ended = chrono::Utc::now();

    // Serialize metadata.
    let meta = Meta {
        job: job_name,
        cmd: cmd.to_vec(),
        pid: child_proc.id(),
        started: started.to_rfc3339(),
        ended: ended.to_rfc3339(),
        exit_code,
    };
    let json = serde_json::to_vec_pretty(&meta)?;
    fs::write(&paths.meta, json)?;

    Ok(())
}
