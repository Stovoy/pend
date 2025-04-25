use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::color::{colors_enabled, COLOR_CODES};
use crate::paths::JobPaths;

/// Public helper mirroring `pend wait <job …>`.
pub(crate) fn wait_jobs(job_names: &[String]) -> io::Result<i32> {
    if job_names.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no job names supplied",
        ));
    }

    if job_names.len() == 1 {
        return wait_single(&job_names[0]);
    }

    wait_interleaved(job_names)
}

// -------------------------------------------------------------------------
// Single-job helper
// -------------------------------------------------------------------------

/// Wait for the given job to finish and replay its captured logs to the
/// current stdout/stderr. Returns the job's exit code.
fn wait_single(job_name: &str) -> io::Result<i32> {
    let paths = JobPaths::new(job_name)?;

    // Defer the existence check until the `.exit` file appears to avoid a
    // race when `pend wait` is invoked immediately after `pend do`.
    let base_delay = std::time::Duration::from_millis(50);
    let max_delay = std::time::Duration::from_secs(2);
    let mut current_delay = base_delay;

    while !paths.exit.exists() {
        std::thread::sleep(current_delay);
        current_delay = std::cmp::min(current_delay * 2, max_delay);
    }

    // Prefer the combined `.log` when present to preserve output order.
    if paths.log.exists() {
        let bytes = fs::read(&paths.log)?;
        io::stdout().write_all(&bytes)?;
    } else {
        if let Ok(bytes) = fs::read(&paths.out) {
            io::stdout().write_all(&bytes)?;
        }
        if let Ok(bytes) = fs::read(&paths.err) {
            io::stderr().write_all(&bytes)?;
        }
    }

    let exit_str = fs::read_to_string(&paths.exit)?;
    let code = exit_str.trim().parse::<i32>().unwrap_or(1);

    emit_summary(job_name, code, &paths.meta)?;

    Ok(code)
}

// -------------------------------------------------------------------------
// Interleaved waiting for multiple jobs
// -------------------------------------------------------------------------

struct JobState {
    name: String,
    log_path: std::path::PathBuf,
    exit_path: std::path::PathBuf,
    log_offset: u64,
    exit_code: Option<i32>,
    color: &'static str,
}

impl JobState {
    fn new(name: &str, color: &'static str) -> io::Result<Self> {
        let effective_color = if colors_enabled() { color } else { "" };
        let paths = JobPaths::new(name)?;
        Ok(Self {
            name: name.to_string(),
            log_path: paths.log,
            exit_path: paths.exit,
            log_offset: 0,
            exit_code: None,
            color: effective_color,
        })
    }

    /// Poll job state once.
    ///
    /// Returns `(finished, progress)` where
    ///  * `finished` signals that the job has terminated (the exit code file
    ///    is present and has been parsed), and
    ///  * `progress` is true when new information became available during this
    ///    poll iteration (either log output or a newly discovered exit code).
    fn poll(&mut self) -> io::Result<(bool /* finished */, bool /* progress */)> {
        // Helper closure reading newly appended bytes from the combined log.
        let read_log = |path: &std::path::Path, offset: &mut u64| -> io::Result<bool> {
            if !path.exists() {
                return Ok(false);
            }

            let size = fs::metadata(path)?.len();
            if size <= *offset {
                return Ok(false);
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
                    io::stdout().write_all(
                        format!(
                            "{}{}{}",
                            self.color,
                            String::from_utf8_lossy(&buffer),
                            reset
                        )
                        .as_bytes(),
                    )?;
                }
                io::stdout().flush()?;
            }

            Ok(!buffer.is_empty())
        };

        let mut progress = read_log(&self.log_path, &mut self.log_offset)?;

        // Check exit code.
        if self.exit_code.is_none() && self.exit_path.exists() {
            let code_str = fs::read_to_string(&self.exit_path)?.trim().to_string();
            self.exit_code = code_str.parse::<i32>().ok();
            progress = true;
        }

        Ok((self.exit_code.is_some(), progress))
    }
}

fn wait_interleaved(job_names: &[String]) -> io::Result<i32> {
    let mut jobs: Vec<JobState> = job_names
        .iter()
        .enumerate()
        .map(|(idx, name)| JobState::new(name, COLOR_CODES[idx % COLOR_CODES.len()]))
        .collect::<Result<_, _>>()?;

    // Basic sanity check: all jobs must have started.
    for job in &jobs {
        if !job.log_path.exists()
            && !job.exit_path.exists()
            && !job.log_path.with_extension("out").exists()
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("unknown job '{}'", job.name),
            ));
        }
    }

    let mut remaining = jobs.len();
    let mut first_error: Option<i32> = None;

    let base_delay = std::time::Duration::from_millis(50);
    let max_delay = std::time::Duration::from_secs(2);
    let mut current_delay = base_delay;

    while remaining > 0 {
        let mut any_progress = false;

        for job in &mut jobs {
            if job.exit_code.is_some()
                && job.log_offset == crate::paths::JobPaths::file_len(&job.log_path)
            {
                continue;
            }

            let (finished, progress) = job.poll()?;
            if progress {
                any_progress = true;
            }

            if finished {
                if let Some(code) = job.exit_code {
                    if code != 0 && first_error.is_none() {
                        first_error = Some(code);
                    }
                }
            }
        }

        remaining = jobs.iter().filter(|j| j.exit_code.is_none()).count();

        if remaining > 0 {
            if any_progress {
                current_delay = base_delay;
            } else {
                current_delay = std::cmp::min(current_delay * 2, max_delay);
            }
            std::thread::sleep(current_delay);
        }
    }

    // Drain any remaining buffered output once more.
    for job in &mut jobs {
        let _ = job.poll()?;
    }

    // Emit summary lines for each job.
    for job in &jobs {
        let meta_path = JobPaths::new(&job.name)?.meta;
        emit_summary(&job.name, job.exit_code.unwrap_or(1), &meta_path)?;
    }

    Ok(first_error.unwrap_or(0))
}

// -------------------------------------------------------------------------
// Shared helpers
// -------------------------------------------------------------------------

fn emit_summary<P: AsRef<std::path::Path>>(
    job_name: &str,
    exit_code: i32,
    meta_path: P,
) -> io::Result<()> {
    let meta_path = meta_path.as_ref();

    let duration_secs = if let Ok(meta_bytes) = fs::read(meta_path) {
        if let Ok(meta_json) = serde_json::from_slice::<serde_json::Value>(&meta_bytes) {
            let started = meta_json.get("started").and_then(|v| v.as_str());
            let ended = meta_json.get("ended").and_then(|v| v.as_str());
            if let (Some(start), Some(end)) = (started, ended) {
                let s = chrono::DateTime::parse_from_rfc3339(start).ok();
                let e = chrono::DateTime::parse_from_rfc3339(end).ok();
                if let (Some(sdt), Some(edt)) = (s, e) {
                    edt.signed_duration_since(sdt).num_seconds().max(0)
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };

    let symbol = if exit_code == 0 { "✓" } else { "✗" };
    println!(
        "{} {} ({} s) – exit {}",
        symbol, job_name, duration_secs, exit_code
    );
    Ok(())
}
