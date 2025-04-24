use std::env;
use std::fs;
use std::io;
use std::path::{PathBuf, Path};

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
    pub out: PathBuf,
    pub err: PathBuf,
    pub exit: PathBuf,
    pub meta: PathBuf,
    pub log: PathBuf,
}

impl JobPaths {
    pub(crate) fn new(job_name: &str) -> io::Result<Self> {
        let root = jobs_root()?;
        Ok(Self {
            out: root.join(format!("{}.out", job_name)),
            err: root.join(format!("{}.err", job_name)),
            exit: root.join(format!("{}.exit", job_name)),
            meta: root.join(format!("{}.json", job_name)),
            log: root.join(format!("{}.log", job_name)),
        })
    }

    pub(crate) fn any_exist(&self) -> bool {
        self.out.exists()
            || self.err.exists()
            || self.exit.exists()
            || self.meta.exists()
            || self.log.exists()
    }

    /// Generic helper returning the file size for the given path or `0` if the
    /// file does not exist.  Used by the waiting helpers.
    pub(crate) fn file_len(path: &Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }
}
