//! Implementation of the `pend wait` sub-command.
//!
//! Waiting can target *one* job (simpler code path) or *multiple* jobs at
//! once. In the latter case the module prints coloured, interleaved output
//! very similar to what `cargo test -- --nocapture` does so that users can
//! follow progress in real time.
//!
//! Efficiency considerations:
//!   • We try to employ the cross-platform [`notify`] crate for near-instant
//!     detection of the `.exit` marker file. When the watcher cannot be
//!     initialised we degrade gracefully to exponential back-off polling.
//!   • For multi-job waits we keep each job's current read position and only
//!     tail the delta since the previous iteration which avoids re-reading
//!     files over and over.
//!
//! The public surface of this module is the [`wait_jobs`] function which is
//! called from `main.rs`.
use anstyle::{AnsiColor, Color, Style};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};

fn color_style(idx: usize) -> Style {
    let color = match idx % 6 {
        0 => AnsiColor::Red,
        1 => AnsiColor::Green,
        2 => AnsiColor::Yellow,
        3 => AnsiColor::Blue,
        4 => AnsiColor::Magenta,
        _ => AnsiColor::Cyan,
    };
    Style::new().fg_color(Some(Color::Ansi(color)))
}

// For efficient change detection we attempt to use a platform file watcher at
// runtime. When that fails (e.g. unsupported platform or too many open
// descriptors) we transparently fall back to the previous exponential back-
// off polling loop so behaviour remains correct albeit slightly less
// efficient.
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::color::colors_enabled;
use crate::paths::JobPaths;

/// Public helper mirroring `pend wait <job …>`.
pub(crate) fn wait_jobs(job_names: &[String]) -> io::Result<i32> {
    if job_names.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no job names supplied",
        ));
    }

    // Before blocking verify that the supplied job names actually refer to
    // *existing* jobs. A completely unknown job would otherwise keep the
    // waiter running forever because none of the expected artifact files
    // (e.g. `<job>.lock`, `<job>.log`, …) will ever show up. We *do not*
    // require that *all* artifact files are present already – creating the
    // first files might race the `pend do` command that launched the job –
    // but at least **one** indicator must exist.
    for name in job_names {
        let paths = JobPaths::new(name)?;

        if !paths.any_exist() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("job '{}' not found", name),
            ));
        }
    }

    if job_names.len() == 1 {
        return wait_single_streaming(&job_names[0]);
    }

    wait_interleaved(job_names)
}

// -------------------------------------------------------------------------
// Single-job helper
// -------------------------------------------------------------------------

/// Wait for the given job to finish and replay its captured logs to the
/// current stdout/stderr. Returns the job's exit code.
fn wait_single_streaming(job_name: &str) -> io::Result<i32> {
    let mut job = JobState::new(job_name, Style::new())?;
    job.style = None; // disable colour for single-job waits

    let mut jobs = vec![job];

    match wait_interleaved_with_watcher(&mut jobs) {
        Ok(code) => Ok(code),
        Err(_e) => wait_interleaved_polling(&mut jobs),
    }
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
    style: Option<anstyle::Style>,
}

impl JobState {
    fn new(name: &str, style: anstyle::Style) -> io::Result<Self> {
        let style_opt = if colors_enabled() { Some(style) } else { None };
        let paths = JobPaths::new(name)?;
        Ok(Self {
            name: name.to_string(),
            log_path: paths.log,
            exit_path: paths.exit,
            log_offset: 0,
            exit_code: None,
            style: style_opt,
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
                if let Some(style) = &self.style {
                    let txt = String::from_utf8_lossy(&buffer);
                    let styled = format!("{}{}{}", style.render(), txt, style.render_reset());
                    io::stdout().write_all(styled.as_bytes())?;
                } else {
                    io::stdout().write_all(&buffer)?;
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
        .map(|(idx, name)| JobState::new(name, color_style(idx)))
        .collect::<Result<_, _>>()?;

    // NOTE: We no longer abort immediately when no artifact files exist yet
    // for a given job. Creation of the first `.log`/`.out` or `.exit` file
    // might race slightly behind the `pend do` command returning. Our
    // watcher-based implementation below will wake up as soon as the job
    // touches any of its artifacts. The legacy polling fallback performs an
    // existence check after a short initial delay instead.

    // Try the watcher-based implementation first. If anything fails we'll
    // transparently fall back to the legacy polling loop.
    match wait_interleaved_with_watcher(&mut jobs) {
        Ok(code) => Ok(code),
        Err(_err) => wait_interleaved_polling(&mut jobs),
    }
}

// -------------------------------------------------------------------------
// Watcher-based implementation
// -------------------------------------------------------------------------

fn wait_interleaved_with_watcher(jobs: &mut [JobState]) -> io::Result<i32> {
    use std::sync::mpsc::channel;
    use std::sync::mpsc::RecvTimeoutError;

    // Determine the common root directory (all artifacts live there).
    let root_dir = jobs
        .first()
        .and_then(|j| j.log_path.parent())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "invalid job path"))?;

    let (event_tx, event_rx) = channel();

    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = event_tx.send(res);
    })
    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    watcher
        .watch(root_dir, RecursiveMode::NonRecursive)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Initial poll flush.
    let mut first_error: Option<i32> = None;
    for job in jobs.iter_mut() {
        let (finished, _progress) = job.poll()?;
        if finished {
            if let Some(code) = job.exit_code {
                if code != 0 && first_error.is_none() {
                    first_error = Some(code);
                }
            }
        }
    }

    // Main event-driven loop.
    while jobs.iter().any(|j| j.exit_code.is_none()) {
        // Wait for any FS event with a generous timeout so we do not block
        // forever in case the watcher misses an update.
        match event_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(_) | Err(RecvTimeoutError::Timeout) => {
                // On any event (or timeout) re-poll all jobs for progress.
                for job in jobs.iter_mut() {
                    let (finished, _progress) = job.poll()?;
                    if finished {
                        if let Some(code) = job.exit_code {
                            if code != 0 && first_error.is_none() {
                                first_error = Some(code);
                            }
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "watcher channel disconnected",
                ));
            }
        }
    }

    // Drain any remaining buffered output.
    for job in jobs.iter_mut() {
        let _ = job.poll()?;
    }

    // Emit summary lines.
    for job in jobs.iter() {
        let meta_path = JobPaths::new(&job.name)?.meta;
        emit_summary(&job.name, job.exit_code.unwrap_or(1), &meta_path)?;
    }

    Ok(first_error.unwrap_or(0))
}

// -------------------------------------------------------------------------
// Legacy polling implementation (fallback)
// -------------------------------------------------------------------------

fn wait_interleaved_polling(jobs: &mut [JobState]) -> io::Result<i32> {
    let mut remaining = jobs.len();
    let mut first_error: Option<i32> = None;

    let base_delay = std::time::Duration::from_millis(50);
    let max_delay = std::time::Duration::from_secs(2);
    let mut current_delay = base_delay;

    while remaining > 0 {
        let mut any_progress = false;

        for job in jobs.iter_mut() {
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

    // Drain remaining output
    for job in jobs.iter_mut() {
        let _ = job.poll()?;
    }

    for job in jobs.iter() {
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
