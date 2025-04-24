use clap::{Parser, Subcommand};

use std::io;

use pend::{do_job, run_worker, wait_jobs};

/// do now, wait later – a tiny job runner
#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    /// Override the location where job artifacts are stored.
    #[arg(long, global = true, value_name = "DIR")]
    dir: Option<std::path::PathBuf>,

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

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    // If a custom directory is given, export it so that library helpers and
    // spawned worker processes pick it up.
    if let Some(dir) = &cli.dir {
        std::env::set_var("PEND_DIR", dir);
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
