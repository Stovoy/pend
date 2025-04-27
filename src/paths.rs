//! Centralised handling of on-disk artifact locations.
//!
//! All files created by *pend* live in a single directory which defaults to
//! `$TMPDIR/pend` but can be overridden through the environment variable
//! `PEND_DIR`. Grouping paths in the [`JobPaths`] struct keeps the logic for
//! constructing and validating those filenames in one place and avoids ad-hoc
//! string formatting throughout the code base.
//!
//! Responsibilities:
//!   • Create / ensure the root directory exists (including environment
//!     override).
//!   • Derive deterministic filenames for the various artifacts
//!     (`.out`, `.err`, `.log`, `.exit`, `.json`, `.lock`, `.signal`).
//!   • Reject paths that would exceed platform path length limits *up front*
//!     so that callers get a clear error instead of an obscure I/O failure
//!     half-way through execution.
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Return the directory into which all job artifacts are written.
///
/// Users can override the default temporary location by setting the
/// `PEND_DIR` environment variable.
/// Determine the directory into which all job artifacts are written and ensure
/// that it exists on the file system.
pub(crate) fn jobs_root() -> io::Result<PathBuf> {
    if let Ok(p) = env::var("PEND_DIR") {
        let path = PathBuf::from(p);
        fs::create_dir_all(&path)?;
        Ok(path)
    } else {
        let mut dir = env::temp_dir();
        dir.push("pend");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// Helper holding all paths used for a given job name.
#[derive(Debug, Clone)]
pub(crate) struct JobPaths {
    pub(crate) out: PathBuf,
    pub(crate) err: PathBuf,
    pub(crate) exit: PathBuf,
    pub(crate) meta: PathBuf,
    pub(crate) log: PathBuf,
    pub(crate) lock: PathBuf,
    pub(crate) signal: PathBuf,
}

impl JobPaths {
    pub(crate) fn new(job_name: &str) -> io::Result<Self> {
        let root = jobs_root()?;
        let paths = Self {
            out: root.join(format!("{}.out", job_name)),
            err: root.join(format!("{}.err", job_name)),
            exit: root.join(format!("{}.exit", job_name)),
            meta: root.join(format!("{}.json", job_name)),
            log: root.join(format!("{}.log", job_name)),
            lock: root.join(format!("{}.lock", job_name)),
            signal: root.join(format!("{}.signal", job_name)),
        };

        paths.assert_paths_within_limit()?;

        Ok(paths)
    }

    /// On construction verify that none of the artifact paths exceeds the
    /// platform‐specific absolute path length limit to avoid cryptic I/O
    /// errors later when we attempt to create the files.
    fn assert_paths_within_limit(&self) -> io::Result<()> {
        #[cfg(windows)]
        const MAX_PATH: usize = 260; // classical Win32 MAX_PATH
        #[cfg(unix)]
        const MAX_PATH: usize = 4096; // typical PATH_MAX on Linux/Unix

        for path in [
            &self.out,
            &self.err,
            &self.exit,
            &self.meta,
            &self.log,
            &self.lock,
            &self.signal,
        ] {
            if let Some(s) = path.to_str() {
                if s.len() >= MAX_PATH {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "artifact path exceeds OS limit ({} > {}): {}",
                            s.len(),
                            MAX_PATH,
                            s
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn any_exist(&self) -> bool {
        self.out.exists()
            || self.err.exists()
            || self.exit.exists()
            || self.meta.exists()
            || self.log.exists()
            || self.signal.exists()
            // Presence of the advisory lock implies a *pend do* invocation
            // is in progress even when the worker has not yet emitted any
            // output files. Including it here avoids a short but racy window
            // in which `pend wait` executed immediately after `pend do` would
            // return `NotFound` despite the job just having been launched.
            || self.lock.exists()
    }

    /// Generic helper returning the file size for the given path or `0` if the
    /// file does not exist. Used by the waiting helpers.
    pub(crate) fn file_len(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
}
