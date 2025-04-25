use clap::{Parser, Subcommand};

use std::io;

mod color;
mod job;
mod paths;
mod wait;
mod worker;
mod tui;

use job::do_job;
use wait::wait_jobs;
use worker::run_worker;

// -------------------------------------------------------------------------
// Helper parsing human-readable size strings like "10M" or "512K" to bytes.
// -------------------------------------------------------------------------

fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("size string is empty".into());
    }

    let mut num_part = String::new();
    let mut unit_part = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !unit_part.is_empty() {
                return Err("invalid size string".into());
            }
            num_part.push(c);
        } else {
            unit_part.push(c);
        }
    }

    let base: u64 = num_part
        .parse()
        .map_err(|_| "invalid numeric component in size string")?;

    let multiplier = match unit_part.to_ascii_uppercase().as_str() {
        "" => 1,
        "K" | "KB" => 1 << 10,
        "M" | "MB" => 1 << 20,
        "G" | "GB" => 1 << 30,
        _ => return Err("unknown size unit".into()),
    };

    Ok(base * multiplier)
}

/// do now, wait later – a tiny job runner
#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    /// Override the location where job artifacts are stored.
    #[arg(long, global = true, value_name = "DIR")]
    dir: Option<std::path::PathBuf>,

    /// Disable ANSI color escapes in multi-job output. Takes precedence over
    /// the `NO_COLOR` environment variable when supplied.
    #[arg(long, global = true)]
    no_color: bool,

    /// Rotate the combined `.log` file once its size exceeds the given limit
    /// (e.g. `10M`, `500K`). The current log becomes `<job>.log.1` and a new
    /// file is started.
    #[arg(long, value_name = "SIZE", global = true)]
    max_log_size: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a job in the background
    Do {
        job_name: String,
        #[arg(required = true, trailing_var_arg = true)]
        cmd: Vec<String>,
    },

    /// Block on one or more jobs and replay their output
    Wait {
        #[arg(required = true)]
        job_names: Vec<String>,
    },

    /// Internal helper – users never call this directly
    #[command(hide = true)]
    Worker {
        job_name: String,
        #[arg(trailing_var_arg = true)]
        cmd: Vec<String>,
    },

    /// Remove job artifacts to free up disk space
    Clean {
        /// Delete *all* artifacts inside the jobs directory. Cannot be used
        /// together with individual job names.
        #[arg(long)]
        all: bool,

        /// One or more job names whose artifacts should be removed.
        #[arg(value_name = "JOB", required_unless_present = "all")]
        jobs: Vec<String>,
    },

    /// Interactive overview of all jobs (press 'q' to quit)
    Tui,
}

// We keep a small wrapper around the previous `main` body so we can format
// errors consistently. Any `io::Error` bubbling up from helper functions is
// intercepted and rendered via its Display implementation instead of the
// rather noisy Debug representation used by Rust’s default panic hook.
fn main() {
    if let Err(err) = try_main() {
        // Use Display, not Debug, for a concise human-friendly message.
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
}

fn try_main() -> io::Result<()> {
    let cli = Cli::parse();

    // If a custom directory is given, export it so that library helpers and
    // spawned worker processes pick it up.
    if let Some(dir) = &cli.dir {
        std::env::set_var("PEND_DIR", dir);
    }

    // Respect the `--no-color` flag by exporting the canonical `NO_COLOR`
    // environment variable so that library helpers and worker processes see
    // the same preference.
    if cli.no_color {
        std::env::set_var("NO_COLOR", "1");
    }

    // Export maximum log size (in bytes) for worker processes.
    if let Some(size_str) = &cli.max_log_size {
        let bytes =
            parse_size(size_str).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        std::env::set_var("PEND_MAX_LOG_SIZE", bytes.to_string());
    }

    match cli.command {
        Commands::Do { job_name, cmd } => do_job(&job_name, &cmd),
        Commands::Wait { job_names } => {
            let code = wait_jobs(&job_names)?;
            std::process::exit(code);
        }
        Commands::Worker { job_name, cmd } => run_worker(&job_name, &cmd),

        Commands::Clean { all, jobs } => {
            use crate::paths::jobs_root;
            use std::fs;

            let root = jobs_root()?;

            // Build list of jobs to remove.
            let targets: Vec<String> = if all {
                // Any file with a known extension indicates presence of a job
                let mut set = std::collections::HashSet::new();
                if let Ok(entries) = fs::read_dir(&root) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            // Strip extension.
                            if let Some((job, ext)) = name.rsplit_once('.') {
                                if matches!(ext, "out" | "err" | "log" | "exit" | "json" | "signal" | "lock")
                                {
                                    set.insert(job.to_string());
                                }
                            }
                        }
                    }
                }
                set.into_iter().collect()
            } else {
                jobs
            };

            if targets.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "no jobs to clean – use --all or supply at least one job name",
                ));
            }

            for job in &targets {
                let paths = crate::paths::JobPaths::new(job)?;
                // Skip deletion if lock file exists and is locked (job running).
                if paths.lock.exists() {
                    use fs2::FileExt;
                    if let Ok(file) = fs::OpenOptions::new().read(true).open(&paths.lock) {
                        if file.try_lock_exclusive().is_err() {
                            eprintln!("warning: job '{job}' appears to be running – skipping");
                            continue;
                        }
                    }
                }

                for p in [
                    &paths.out,
                    &paths.err,
                    &paths.log,
                    &paths.exit,
                    &paths.meta,
                    &paths.signal,
                    &paths.lock,
                ] {
                    let _ = fs::remove_file(p);
                }
            }
            Ok(())
        }

        Commands::Tui => {
            crate::tui::run_tui()?;
            Ok(())
        }
    }
}
