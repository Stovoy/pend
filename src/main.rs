use clap::{Parser, Subcommand};

use std::io;

mod color;
mod job;
mod paths;
mod wait;
mod worker;

use job::do_job;
use wait::wait_jobs;
use worker::run_worker;

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

    match cli.command {
        Commands::Do { job_name, cmd } => do_job(&job_name, &cmd),
        Commands::Wait { job_names } => {
            let code = wait_jobs(&job_names)?;
            std::process::exit(code);
        }
        Commands::Worker { job_name, cmd } => run_worker(&job_name, &cmd),
    }
}
