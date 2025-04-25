use std::io;

use crate::paths::JobPaths;
use fs2::FileExt;
use std::fs::OpenOptions;

/// Public helper equivalent to `pend do <job> <cmd …>`.
pub(crate) fn do_job(job_name: &str, cmd: &[String]) -> io::Result<()> {
    if job_name.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name cannot be empty",
        ));
    }

    // ------------------------------------------------------------------
    // Job-name validation
    //
    // Rules (see TODO.md step 4):
    //   • ASCII letters, digits, dash, underscore, and single dots are allowed
    //   • No leading dot
    //   • No repeated dots ("..")
    //   • Maximum length 100 codepoints
    //   • No path separators
    //   • Must be in Unicode NFC normal form (if non-ASCII)
    // ------------------------------------------------------------------

    // Quick path-separator rejection prevents directory traversal.
    if job_name.contains('/') || job_name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name must not contain path separators",
        ));
    }

    // Length limit.
    if job_name.chars().count() > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name must not exceed 100 characters",
        ));
    }

    // No leading dot or repeated dots.
    if job_name.starts_with('.') || job_name.contains("..") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name must not start with a dot or contain repeated dots",
        ));
    }

    // Allowed ASCII character set plus unrestricted Unicode in NFC form.
    if !job_name.chars().all(|c| {
        if c.is_ascii() {
            c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
        } else {
            // Non-ASCII characters are permitted as long as the overall
            // string is NFC.  We accept any non-control Unicode scalar.
            !c.is_control()
        }
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name contains invalid characters",
        ));
    }

    // Enforce NFC normalization to avoid duplicate names referring to the
    // same canonical representation.
    use unicode_normalization::UnicodeNormalization;
    if job_name.nfc().collect::<String>() != job_name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "job name must be Unicode NFC normalised",
        ));
    }

    if cmd.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command cannot be empty",
        ));
    }

    let paths = JobPaths::new(job_name)?;

    // ------------------------------------------------------------------
    // Advisory lock to guard against concurrent `pend do` invocations for
    // the *same* job name. We create (or open) a lightweight `.lock` file
    // next to the other artifacts and attempt to obtain a non-blocking
    // exclusive lock. If another process already holds the lock we surface
    // a user-friendly error immediately instead of racing to create the
    // artifact files only to fail later on.
    // ------------------------------------------------------------------

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

    if paths.any_exist() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("job '{job_name}' already exists"),
        ));
    }

    super::worker::spawn_worker(job_name, cmd)
}
