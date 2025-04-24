use std::io;

use crate::paths::JobPaths;

/// Public helper equivalent to `pend do <job> <cmd …>`.
pub fn do_job(job_name: &str, cmd: &[String]) -> io::Result<()> {
    if job_name.trim().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "job name cannot be empty"));
    }

    // Reject names that could escape the jobs directory or clash with
    // neighbouring files. We only permit ASCII letters, digits, `-` and `_`.
    if !job_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name contains invalid characters",
        ));
    }

    // Disallow path separators outright as an extra safeguard.
    if job_name.contains('/') || job_name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name must not contain path separators",
        ));
    }

    if cmd.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "command cannot be empty"));
    }

    let paths = JobPaths::new(job_name)?;
    if paths.any_exist() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("job '{job_name}' already exists"),
        ));
    }

    super::worker::spawn_worker(job_name, cmd)
}
