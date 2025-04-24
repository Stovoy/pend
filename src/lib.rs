use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use serde::Serialize;

/// Return the directory into which all job artifacts are written.
///
/// Users can override the default temporary location by setting the
/// `PEND_DIR` environment variable.
pub fn jobs_root() -> PathBuf {
    if let Ok(p) = env::var("PEND_DIR") {
        let path = PathBuf::from(p);
        fs::create_dir_all(&path).ok();
        path
    } else {
        let mut dir = env::temp_dir();
        dir.push("pend");
        fs::create_dir_all(&dir).ok();
        dir
    }
}

/// Helper holding all paths used for a given job name.
#[derive(Debug, Clone)]
struct JobPaths {
    out: PathBuf,
    err: PathBuf,
    exit: PathBuf,
    meta: PathBuf,
    log: PathBuf,
}

impl JobPaths {
    fn new(job_name: &str) -> Self {
        let root = jobs_root();
        Self {
            out: root.join(format!("{}.out", job_name)),
            err: root.join(format!("{}.err", job_name)),
            exit: root.join(format!("{}.exit", job_name)),
            meta: root.join(format!("{}.json", job_name)),
            log: root.join(format!("{}.log", job_name)),
        }
    }

    fn any_exist(&self) -> bool {
        self.out.exists()
            || self.err.exists()
            || self.exit.exists()
            || self.meta.exists()
            || self.log.exists()
    }
}

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
/// command and records artifacts. This helper is invoked by the public
/// [`do_job`] function.
fn spawn_worker(job_name: &str, cmd: &[String]) -> io::Result<()> {
    let exe_path = env::current_exe()?;
    let mut worker_cmd = Command::new(&exe_path);
    worker_cmd.arg("worker").arg(job_name).arg("--");

    worker_cmd.args(cmd);

    // Detach: we do *not* inherit stdin/stdout/stderr to avoid mixing logs.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Start new session so Ctrl-C in parent shell does not propagate.
        // `pre_exec` is `unsafe` because the closure runs in the forked
        // process *before* `exec`, where very few operations are allowed.
        unsafe {
            worker_cmd.pre_exec(|| {
                // Create a new session so the worker does not receive the
                // parent's signals (e.g. Ctrl-C).
                libc::setsid();
                Ok(())
            });
        }
    }

    worker_cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());

    // On Windows, start_new_session is available from Rust 1.63+.
    worker_cmd.spawn()?;
    Ok(())
}

/// Public helper equivalent to `pend do <job> <cmd …>`.
pub fn do_job(job_name: &str, cmd: &[String]) -> io::Result<()> {
    if job_name.trim().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "job name cannot be empty"));
    }

    // Reject names that could escape the jobs directory or clash with
    // neighbouring files. We only permit ASCII letters, digits, `-` and `_`
    // and disallow path separators outright.
    if !job_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name contains invalid characters",
        ));
    }

    // A path separator slipping through `is_ascii_alphanumeric` check on
    // certain platforms would be caught here as an additional safety net.
    if job_name.contains('/') || job_name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name must not contain path separators",
        ));
    }
    if cmd.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "command cannot be empty"));
    }

    let paths = JobPaths::new(job_name);
    if paths.any_exist() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("job '{job_name}' already exists"),
        ));
    }

    spawn_worker(job_name, cmd)
}

/// Internal function executed by the *worker* sub-command.
/// Entry point used by the hidden `worker` CLI subcommand. Not considered part
/// of the stable public API but exported so the binary can invoke it.
pub fn run_worker(job_name: &str, cmd: &[String]) -> io::Result<()> {
    let paths = JobPaths::new(job_name);


    // We'll capture stdout/stderr via pipes so that we can merge them while
    // still writing dedicated .out / .err files.

    let started = chrono::Utc::now();

    let mut child_proc = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout_pipe = child_proc
        .stdout
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to capture stdout"))?;
    let stderr_pipe = child_proc
        .stderr
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "failed to capture stderr"))?;

    let mut out_file = File::create(&paths.out)?;
    let mut err_file = File::create(&paths.err)?;
    let mut log_file = File::create(&paths.log)?;

    #[derive(Clone, Copy)]
    enum StreamKind {
        Stdout,
        Stderr,
    }

    use std::sync::mpsc::{self, Sender};
    let (tx, rx) = mpsc::channel::<(StreamKind, Vec<u8>)>();
    fn spawn_reader<R: Read + Send + 'static>(
        kind: StreamKind,
        reader: R,
        tx: Sender<(StreamKind, Vec<u8>)>,
    ) {
        std::thread::spawn(move || {
            let mut buf_reader = std::io::BufReader::new(reader);
            let mut chunk = [0u8; 4096];
            loop {
                match buf_reader.read(&mut chunk) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        tx.send((kind, chunk[..n].to_vec())).ok();
                    }
                    Err(_) => break,
                }
            }
        });
    }

    spawn_reader(StreamKind::Stdout, stdout_pipe, tx.clone());
    spawn_reader(StreamKind::Stderr, stderr_pipe, tx.clone());

    drop(tx); // close original sender in parent

    for (kind, chunk) in rx.iter() {
        match kind {
            StreamKind::Stdout => {
                out_file.write_all(&chunk)?;
                log_file.write_all(&chunk)?;
            }
            StreamKind::Stderr => {
                err_file.write_all(&chunk)?;
                log_file.write_all(&chunk)?;
            }
        }
    }

    let status = child_proc.wait()?;

    let exit_code = status.code().unwrap_or(1);

    let ended = chrono::Utc::now();

    // Write exit code file.
    fs::write(&paths.exit, format!("{}\n", exit_code))?;

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

/// Wait for the given job to finish and replay its captured logs to the
/// current stdout/stderr. Returns the job's exit code.
fn wait_single(job_name: &str) -> io::Result<i32> {
    let paths = JobPaths::new(job_name);

    // The worker process may not have had a chance to create its artifact
    // files yet when `pend wait` is invoked immediately after `pend do`.
    // Avoid a race where we mistake a just-spawned job for an unknown one by
    // deferring the existence check.  We now rely on the appearance of the
    // `.exit` file which is written by the worker once the job completes.

    // Poll for the .exit file.
    loop {
        if paths.exit.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Replay output in original order if combined `.log` exists.
    if paths.log.exists() {
        let bytes = fs::read(&paths.log)?;
        io::stdout().write_all(&bytes)?;
    } else {
        // Fallback to separate .out / .err files.
        if let Ok(bytes) = fs::read(&paths.out) {
            io::stdout().write_all(&bytes)?;
        }
        if let Ok(bytes) = fs::read(&paths.err) {
            io::stderr().write_all(&bytes)?;
        }
    }

    let exit_str = fs::read_to_string(&paths.exit)?;
    Ok(exit_str.trim().parse::<i32>().unwrap_or(1))
}

/// Public helper mirroring `pend wait <job …>`.
pub fn wait_jobs(job_names: &[String]) -> io::Result<i32> {
    if job_names.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no job names supplied"));
    }

    if job_names.len() == 1 {
        return wait_single(&job_names[0]);
    }

    wait_interleaved(job_names)
}

// -------------------------------------------------------------------------
// Interleaved waiting for multiple jobs
// -------------------------------------------------------------------------

const COLOR_CODES: [&str; 6] = [
    "\x1b[31m", // red
    "\x1b[32m", // green
    "\x1b[33m", // yellow
    "\x1b[34m", // blue
    "\x1b[35m", // magenta
    "\x1b[36m", // cyan
];

fn colors_enabled() -> bool {
    // De-facto standard: if the `NO_COLOR` environment variable is present
    // (with any value), programs should avoid emitting ANSI escape sequences.
    std::env::var_os("NO_COLOR").is_none()
}

struct JobState {
    name: String,
    log_path: PathBuf,
    exit_path: PathBuf,
    log_offset: u64,
    exit_code: Option<i32>,
    color: &'static str,
}

impl JobState {
    fn new(name: &str, color: &'static str) -> Self {
        let effective_color = if colors_enabled() { color } else { "" };
        let paths = JobPaths::new(name);
        Self {
            name: name.to_string(),
            log_path: paths.log,
            exit_path: paths.exit,
            log_offset: 0,
            exit_code: None,
            color: effective_color,
        }
    }

    fn poll(&mut self) -> io::Result<bool> {
        // Helper closure for reading new chunk from log file.
        let read_log = |path: &Path, offset: &mut u64| -> io::Result<()> {
            if !path.exists() {
                return Ok(());
            }

            let size = fs::metadata(path)?.len();
            if size <= *offset {
                return Ok(());
            }

            let mut file = File::open(path)?;
            file.seek(SeekFrom::Start(*offset))?;

            let mut buffer = Vec::with_capacity((size - *offset) as usize);
            file.read_to_end(&mut buffer)?;
            *offset = size;

            if !buffer.is_empty() {
                if self.color.is_empty() {
                    io::stdout().write_all(&buffer)?;
                } else {
                    let reset = "\x1b[0m";
                    io::stdout()
                        .write_all(format!("{}{}{}", self.color, String::from_utf8_lossy(&buffer), reset).as_bytes())?;
                }
                io::stdout().flush()?;
            }

            Ok(())
        };

        read_log(&self.log_path, &mut self.log_offset)?;

        // Check exit code.
        if self.exit_code.is_none() && self.exit_path.exists() {
            let code_str = fs::read_to_string(&self.exit_path)?.trim().to_string();
            self.exit_code = code_str.parse::<i32>().ok();
        }

        Ok(self.exit_code.is_some())
    }
}

fn wait_interleaved(job_names: &[String]) -> io::Result<i32> {
    // Assign colors cyclically.
    let mut jobs: Vec<JobState> = job_names
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            JobState::new(name, COLOR_CODES[idx % COLOR_CODES.len()])
        })
        .collect();

    // Ensure all jobs have started; otherwise return error early.
    for job in &jobs {
        if !job.log_path.exists() && !job.exit_path.exists() && !job.log_path.with_extension("out").exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("unknown job '{}'", job.name),
            ));
        }
    }

    let mut remaining = jobs.len();
    let mut first_error: Option<i32> = None;

    while remaining > 0 {
        for job in &mut jobs {
            if job.exit_code.is_some()
                && job.log_offset == fs::metadata(&job.log_path).map(|m| m.len()).unwrap_or(0)
            {
                // already finished and consumed
                continue;
            }

            let finished = job.poll()?;
            if finished {
                if let Some(code) = job.exit_code {
                    if code != 0 && first_error.is_none() {
                        first_error = Some(code);
                    }
                }
            }
        }

        // Update remaining.
        remaining = jobs
            .iter()
            .filter(|j| j.exit_code.is_none())
            .count();

        if remaining > 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Ensure we drained final output.
    for job in &mut jobs {
        job.poll()?;
    }

    Ok(first_error.unwrap_or(0))
}

/// Helper for tests: execute a command and obtain its [`ExitStatus`].
#[allow(dead_code)]
fn run_command(program: &Path, args: &[&str]) -> io::Result<ExitStatus> {
    Command::new(program).args(args).status()
}
