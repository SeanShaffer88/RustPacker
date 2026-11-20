//! Scratch directories for tests.
//!
//! Test binaries run in parallel, and several of them share a PID namespace
//! (cargo reuses the harness for `--test` targets, and two `cargo test`
//! invocations can overlap). Naming a directory after `process::id()` alone
//! therefore collides: two tests in the same binary get one directory and
//! delete each other's files mid-run.
//!
//! [`ScratchDir`] fixes that with a process-wide counter, and it removes the
//! directory on drop so a failing assertion does not leave debris behind.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// A uniquely named directory under the system temp dir, deleted on drop.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Create a fresh empty directory named `<prefix>-<pid>-<n>`.
    ///
    /// `prefix` only exists to make failures readable; uniqueness comes from
    /// the pid and the counter.
    pub fn new(prefix: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()));
        // A previous run may have been killed before its drop ran.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch dir");
        Self { path }
    }

    /// Create a fresh directory and return a path inside it (`name`).
    pub fn with_file(prefix: &str, name: &str) -> PathBuf {
        Self::new(prefix).path.join(name)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `self.path().join(part)`.
    pub fn join<P: AsRef<Path>>(&self, part: P) -> PathBuf {
        self.path.join(part)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_within_one_process() {
        let a = ScratchDir::new("rustpacker-scratch-test");
        let b = ScratchDir::new("rustpacker-scratch-test");
        assert_ne!(a.path(), b.path());
        assert!(a.path().is_dir() && b.path().is_dir());
    }

    #[test]
    fn drop_removes_the_directory() {
        let path = {
            let dir = ScratchDir::new("rustpacker-scratch-drop");
            let p = dir.path().to_path_buf();
            assert!(p.is_dir());
            p
        };
        assert!(!path.exists(), "scratch dir should be gone after drop");
    }

    #[test]
    fn with_file_returns_a_path_inside_an_existing_dir() {
        let path = ScratchDir::with_file("rustpacker-scratch-file", "out.bin");
        assert!(path.parent().expect("parent").is_dir());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }
}
