//! A small, append-only record of what the installer did.
//!
//! AUDIT M7: the wizard reported failures only through message boxes, so a
//! user reporting "it failed" left nothing behind to diagnose with. This is
//! deliberately not a logging framework: an installer's job is to write a few
//! dozen lines describing its own steps, and a dependency-free implementation
//! keeps the stub small and its behaviour obvious.
//!
//! The log is:
//!
//! * **append-only** — each [`InstallLog::note`] adds one line, so a crash
//!   part-way through leaves a record of the steps that had completed;
//! * **bounded** — the file is rotated to `.1` once it exceeds
//!   [`MAX_LOG_BYTES`], so repeated installs cannot grow it without limit;
//! * **best effort** — every method returns whether the write succeeded and
//!   never panics, because a logging problem must never fail an install;
//! * **timestamped without a clock dependency** — elapsed milliseconds since
//!   the log was created, which is what actually helps when reading a
//!   sequence of steps (`+1832ms`), rather than wall-clock times that need a
//!   timezone to interpret.
//!
//! Nothing here touches the filesystem until [`InstallLog::open`] is called,
//! so the entire format is unit-testable on any platform.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Rotate once the log exceeds this many bytes.
///
/// 256 KiB is thousands of lines — far more than one install produces — so
/// rotation only ever happens after many installs have shared a file.
pub const MAX_LOG_BYTES: u64 = 256 * 1024;

/// The default file name, written beside the install journal.
pub const LOG_FILE_NAME: &str = "install.log";

/// One line of the log, before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Milliseconds since the log was opened.
    pub elapsed_ms: u64,
    pub message: String,
}

/// Format one entry as a line, without a trailing newline.
///
/// Kept separate from [`InstallLog`] so the format is testable without a
/// filesystem: the separator, the timestamp padding and the sanitisation all
/// live here.
pub fn format_entry(elapsed_ms: u64, message: &str) -> String {
    // Newlines inside a message would split one entry across lines and make
    // the log ambiguous, so they become spaces.
    let single_line: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut line = String::with_capacity(single_line.len() + 16);
    let _ = write!(line, "+{elapsed_ms}ms\t{}", single_line.trim());
    line
}

/// An open install log.
pub struct InstallLog {
    path: PathBuf,
    file: Option<File>,
    started: Instant,
    /// Set once writing has failed, so a full disk produces one warning
    /// rather than one per line.
    failed: bool,
    written: u64,
}

impl std::fmt::Debug for InstallLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `File` is Debug, but printing an open handle says nothing useful;
        // the path and whether writing works is what a failure message needs.
        f.debug_struct("InstallLog")
            .field("path", &self.path)
            .field("open", &self.file.is_some())
            .field("failed", &self.failed)
            .finish()
    }
}

impl InstallLog {
    /// Open (or create) the log at `path`, recording the header line.
    ///
    /// A failure to open is reported through the returned `Result`, but the
    /// caller is expected to carry on regardless: [`InstallLog::to_stderr`]
    /// gives a working log when the file cannot be written.
    pub fn open(path: &Path) -> Result<Self, String> {
        // The log lives in the install folder, which usually does not
        // exist yet when the installer starts: create it, or every fresh
        // install silently ends up with no log at all. Best effort — a
        // failure here is reported by the open below, as before.
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_LOG_BYTES {
            // Keep exactly one previous generation. Rotation failing is not
            // fatal: appending to an oversized log still beats no log.
            let mut backup = path.as_os_str().to_owned();
            backup.push(".1");
            let _ = std::fs::rename(path, PathBuf::from(backup));
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("could not open {}: {e}", path.display()))?;
        let mut log = Self {
            path: path.to_path_buf(),
            file: Some(file),
            started: Instant::now(),
            failed: false,
            written: 0,
        };
        // The header goes through `write_line` directly: it is not a step,
        // so it has no elapsed time and must not look like one.
        log.write_line("---- RustPacker install log ----");
        Ok(log)
    }

    /// A log that writes nothing but reports success, for `--silent` runs and
    /// for callers that only care about the steps they take.
    pub fn disabled() -> Self {
        Self {
            path: PathBuf::new(),
            file: None,
            started: Instant::now(),
            failed: false,
            written: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True when writes are being accepted.
    pub fn is_writable(&self) -> bool {
        self.file.is_some() && !self.failed
    }

    /// Append one line. Returns whether it reached the file.
    pub fn note(&mut self, message: &str) -> bool {
        let elapsed = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.write_line(&format_entry(elapsed, message))
    }

    /// Append one line without a timestamp, used for the header and for
    /// pre-formatted blocks.
    fn write_line(&mut self, line: &str) -> bool {
        let Some(file) = self.file.as_mut() else {
            return false;
        };
        if self.failed {
            return false;
        }
        // A single write of the line plus its newline: an installer that is
        // killed mid-log loses at most this line.
        let mut buf = Vec::with_capacity(line.len() + 2);
        buf.extend_from_slice(line.as_bytes());
        buf.extend_from_slice(b"\r\n"); // CRLF so Notepad shows it correctly.
        match file.write_all(&buf) {
            Ok(()) => {
                self.written += buf.len() as u64;
                true
            }
            Err(e) => {
                self.failed = true;
                eprintln!("install log ({}) failed: {e}", self.path.display());
                false
            }
        }
    }

    /// Record an error, marking it so it is findable with a grep.
    pub fn error(&mut self, message: &str) -> bool {
        self.note(&format!("ERROR {message}"))
    }

    /// Record a warning.
    pub fn warn(&mut self, message: &str) -> bool {
        self.note(&format!("WARN  {message}"))
    }

    /// Flush and record how long the run took. Called once at the end.
    pub fn finish(&mut self, outcome: &str) {
        // Like the header, the footer is a separator rather than a step, so
        // it goes through `write_line` and carries no timestamp.
        self.write_line(&format!("---- finished: {outcome} ----"));
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
            let _ = file.sync_data();
        }
    }

    /// Total bytes written by this handle, for tests and diagnostics.
    pub fn bytes_written(&self) -> u64 {
        self.written
    }
}

/// Convenience: an `InstallLog` that writes to stderr only.
///
/// Used when the install directory is not writable yet (the very first steps
/// happen before the folder exists), so early failures still leave a trace in
/// the console for `--silent` runs launched from a terminal.
#[derive(Debug)]
pub struct StderrLog {
    started: Instant,
    lines: Vec<String>,
}

impl Default for StderrLog {
    fn default() -> Self {
        Self::new()
    }
}

impl StderrLog {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            lines: Vec::new(),
        }
    }

    pub fn note(&mut self, message: &str) {
        let elapsed = self.started.elapsed();
        self.push(elapsed, message);
    }

    pub fn push(&mut self, elapsed: Duration, message: &str) {
        let line = format_entry(elapsed.as_millis().min(u64::MAX as u128) as u64, message);
        eprintln!("{line}");
        self.lines.push(line);
    }

    /// The lines written so far, so they can be copied into a real log once
    /// one becomes available.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ScratchDir;

    #[test]
    fn entries_are_single_lines_with_a_timestamp() {
        let line = format_entry(0, "creating install folder");
        assert_eq!(line, "+0ms\tcreating install folder");
        // Embedded newlines would split one entry across lines.
        let line = format_entry(1234, "first\nsecond\ttabbed");
        assert!(!line.contains('\n'), "{line:?}");
        assert!(
            !line.contains('\t') || line.matches('\t').count() == 1,
            "{line:?}"
        );
        assert!(line.starts_with("+1234ms\t"));
        // Surrounding whitespace is trimmed so entries line up.
        assert_eq!(format_entry(5, "  spaced  "), "+5ms\tspaced");
    }

    #[test]
    fn note_appends_in_order_and_reports_success() {
        let dir = ScratchDir::new("rustpacker-log-test");
        let path = dir.join(LOG_FILE_NAME);
        let mut log = InstallLog::open(&path).expect("open");
        assert!(log.is_writable());
        assert!(log.note("first step"));
        assert!(log.note("second step"));
        log.error("something broke");
        log.warn("something odd");
        log.finish("installed");
        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "---- RustPacker install log ----");
        assert!(!lines[0].starts_with('+'), "the header is not a step");
        assert!(lines[1].ends_with("first step"), "{:?}", lines[1]);
        assert!(lines[2].ends_with("second step"));
        assert!(lines[3].ends_with("ERROR something broke"));
        assert!(lines[4].ends_with("WARN  something odd"));
        assert_eq!(
            *lines.last().expect("last"),
            "---- finished: installed ----"
        );
        // Every step between the header and the footer carries a timestamp.
        for line in &lines[1..lines.len() - 1] {
            assert!(line.starts_with('+'), "no timestamp: {line:?}");
        }
        // CRLF, so Notepad does not show one long line.
        assert!(text.contains("\r\n"));
        assert!(log.bytes_written() > 0);
    }

    #[test]
    fn open_creates_a_missing_parent_folder() {
        // Regression: the wizard opens the log before the install folder
        // exists, so a fresh install used to end up with no log at all.
        let dir = ScratchDir::new("rustpacker-log-parent");
        let path = dir.join("not").join("created").join(LOG_FILE_NAME);
        let mut log = InstallLog::open(&path).expect("open should create parents");
        log.note("first line");
        drop(log);
        let text = std::fs::read_to_string(&path).expect("log should exist");
        assert!(text.contains("first line"), "{text}");
    }

    #[test]
    fn a_second_open_appends_rather_than_truncating() {
        let dir = ScratchDir::new("rustpacker-log-append");
        let path = dir.join(LOG_FILE_NAME);
        {
            let mut log = InstallLog::open(&path).expect("open");
            log.note("run one");
        }
        {
            let mut log = InstallLog::open(&path).expect("reopen");
            log.note("run two");
        }
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("run one"), "{text}");
        assert!(text.contains("run two"), "{text}");
        // Two headers, one per run: the log shows when each install began.
        assert_eq!(text.matches("---- RustPacker install log ----").count(), 2);
    }

    #[test]
    fn an_oversized_log_is_rotated_to_one_backup() {
        let dir = ScratchDir::new("rustpacker-log-rotate");
        let path = dir.join(LOG_FILE_NAME);
        // Pre-seed a log just over the limit.
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).expect("seed");
        let mut log = InstallLog::open(&path).expect("open");
        log.note("after rotation");
        let mut backup = path.as_os_str().to_owned();
        backup.push(".1");
        let backup = PathBuf::from(backup);
        assert!(backup.exists(), "the oversized log should be kept as .1");
        let fresh = std::fs::read_to_string(&path).expect("read");
        assert!(
            (fresh.len() as u64) < MAX_LOG_BYTES,
            "the new log should be small, was {} bytes",
            fresh.len()
        );
        assert!(fresh.contains("after rotation"));
    }

    #[test]
    fn an_unwritable_path_reports_failure_without_panicking() {
        let dir = ScratchDir::new("rustpacker-log-fail");
        // A directory where the file belongs: open must fail cleanly.
        let path = dir.join("as-a-directory");
        std::fs::create_dir_all(&path).expect("mkdir");
        let err = InstallLog::open(&path).expect_err("must fail");
        assert!(err.contains("could not open"), "{err}");
    }

    #[test]
    fn a_disabled_log_accepts_notes_and_writes_nothing() {
        let mut log = InstallLog::disabled();
        assert!(!log.is_writable());
        // The return value says the line was not written, but nothing panics
        // and the caller does not need to special-case it.
        assert!(!log.note("ignored"));
        assert_eq!(log.bytes_written(), 0);
        assert!(log.path().as_os_str().is_empty());
    }

    #[test]
    fn stderr_log_keeps_its_lines_for_later() {
        let mut log = StderrLog::new();
        log.push(Duration::from_millis(7), "early step");
        log.note("later step");
        assert_eq!(log.lines().len(), 2);
        assert_eq!(log.lines()[0], "+7ms\tearly step");
        assert!(log.lines()[1].ends_with("later step"));
    }
}
