//! Transactional install journal.
//!
//! Every change an install makes is recorded here in order. Rollback and
//! uninstall both walk the journal in reverse, so nothing the installer
//! created is left behind and nothing it replaced is lost silently:
//! overwritten files and replaced registry values keep their previous
//! data in the journal entry.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum JournalAction {
    CreateDir {
        path: String,
    },
    CreateFile {
        path: String,
    },
    /// A file that existed before install; `backup_name` is the copy kept
    /// beside the journal until uninstall/rollback completes.
    OverwriteFile {
        path: String,
        backup_name: String,
    },
    CreateShortcut {
        path: String,
    },
    RegistryValue {
        root: String,
        key: String,
        name: String,
        /// Previous value data, when the value already existed.
        #[serde(default)]
        previous_value: Option<String>,
    },
    /// A value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
    RunEntry {
        name: String,
    },
    /// The Apps & Features (Add/Remove Programs) registration key.
    ArpEntry {
        root: String,
        key: String,
    },
    PathEntry {
        scope: String,
        added_dir: String,
    },
    FileAssociation {
        extension: String,
        prog_id: String,
        /// Whether these keys already existed before install. When
        /// true, uninstall must not delete them (they held user data).
        #[serde(default)]
        previous_existed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub action: JournalAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallJournal {
    pub schema_version: u32,
    pub app_id: String,
    pub app_name: String,
    pub version: String,
    pub install_dir: String,
    /// `per-user` or `per-machine` — decides which ARP root uninstall uses.
    pub scope: String,
    pub entries: Vec<JournalEntry>,
}

impl Default for InstallJournal {
    fn default() -> Self {
        Self {
            schema_version: 1,
            app_id: String::new(),
            app_name: String::new(),
            version: String::new(),
            install_dir: String::new(),
            scope: "per-user".into(),
            entries: Vec::new(),
        }
    }
}

impl InstallJournal {
    pub fn new(
        app_id: &str,
        app_name: &str,
        version: &str,
        install_dir: &str,
        scope: &str,
    ) -> Self {
        Self {
            app_id: app_id.into(),
            app_name: app_name.into(),
            version: version.into(),
            install_dir: install_dir.into(),
            scope: scope.into(),
            ..Self::default()
        }
    }

    pub fn record(&mut self, action: JournalAction) {
        self.entries.push(JournalEntry { action });
    }

    pub fn record_file(&mut self, path: &str) {
        self.record(JournalAction::CreateFile { path: path.into() });
    }

    pub fn record_dir(&mut self, path: &str) {
        self.record(JournalAction::CreateDir { path: path.into() });
    }

    pub fn record_run_entry(&mut self, name: &str) {
        self.record(JournalAction::RunEntry { name: name.into() });
    }

    /// Entries in the order they must be undone (reverse of creation).
    pub fn rollback_plan(&self) -> Vec<&JournalAction> {
        self.entries.iter().map(|e| &e.action).rev().collect()
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("could not serialise journal: {e}"))
    }

    /// Basic sanity checks for a loaded journal. Rejects journals
    /// from a newer schema and journals without an install directory
    /// (which would make confinement checks meaningless).
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version == 0 || self.schema_version > 1 {
            return Err(format!(
                "install journal schema_version {} is not supported (expected 1)",
                self.schema_version
            ));
        }
        if self.install_dir.trim().is_empty() {
            return Err("install journal has an empty install_dir".into());
        }
        Ok(())
    }

    /// On-disk location of an `OverwriteFile` backup, or `None` when the
    /// recorded `backup_name` is not a bare file name.
    ///
    /// A tampered journal must not be able to point a restore or delete
    /// at `..\..\Windows\System32` or an absolute path, so the name is
    /// validated with the same rules as `app.main_exe`.
    pub fn backup_path(&self, backup_name: &str) -> Option<std::path::PathBuf> {
        if !crate::schema::is_bare_filename(backup_name) {
            return None;
        }
        Some(std::path::Path::new(&self.install_dir).join(backup_name))
    }

    /// True when `path` is the install directory itself or strictly
    /// inside it (lexical, case-insensitive, `/`/`\\` agnostic).
    /// The uninstaller must refuse to act on any journal path for
    /// which this is false.
    pub fn is_path_confined(&self, path: &str) -> bool {
        fn normalise(p: &str) -> String {
            p.replace('\\', "/")
                .trim_end_matches('/')
                .to_ascii_lowercase()
        }
        let base = normalise(&self.install_dir);
        let target = normalise(path);
        if base.is_empty() || target.is_empty() {
            return false;
        }
        target == base || target.starts_with(&format!("{base}/"))
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let journal: InstallJournal =
            serde_json::from_str(text).map_err(|e| format!("invalid install journal: {e}"))?;
        journal.validate()?;
        Ok(journal)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let text = self.to_json()?;
        // Atomic write: temp file in the same folder, fsync, then rename.
        let tmp = path.with_extension("json.tmp");
        let write_result = (|| -> Result<(), String> {
            std::fs::write(&tmp, text).map_err(|e| format!("could not write journal: {e}"))?;
            // Best-effort durability: flush the file to disk before the
            // rename so a crash cannot leave an empty journal behind.
            if let Ok(file) = std::fs::OpenOptions::new().read(true).open(&tmp) {
                let _ = file.sync_all();
            }
            std::fs::rename(&tmp, path).map_err(|e| format!("could not save journal: {e}"))?;
            // Best-effort directory fsync (not available on all platforms).
            if let Some(parent) = path.parent() {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
            Ok(())
        })();
        if write_result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        write_result
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read journal {}: {e}", path.display()))?;
        Self::from_json(&text)
    }
}

/// File name of the journal inside the install directory.
pub const JOURNAL_FILE_NAME: &str = "install.journal.json";
/// File name of the generated uninstaller inside the install directory.
pub const UNINSTALLER_FILE_NAME: &str = "Uninstall.exe";

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InstallJournal {
        let mut j = InstallJournal::new(
            "MyApp",
            "My App",
            "1.0.0",
            "C:\\Users\\x\\Programs\\MyApp",
            "per-user",
        );
        j.record_dir("C:\\Users\\x\\Programs\\MyApp");
        j.record_file("C:\\Users\\x\\Programs\\MyApp\\MyApp.exe");
        j.record_run_entry("MyApp");
        j
    }

    #[test]
    fn rollback_plan_is_reverse_order() {
        let j = sample();
        let plan = j.rollback_plan();
        assert!(matches!(plan[0], JournalAction::RunEntry { .. }));
        assert!(matches!(plan[1], JournalAction::CreateFile { .. }));
        assert!(matches!(plan[2], JournalAction::CreateDir { .. }));
    }

    #[test]
    fn json_round_trip() {
        let j = sample();
        let text = j.to_json().expect("serialise");
        let back = InstallJournal::from_json(&text).expect("parse");
        assert_eq!(back, j);
    }

    #[test]
    fn backup_names_must_be_bare_filenames() {
        let j = sample();
        let ok = j
            .backup_path("install.journal.json.abc123.bak")
            .expect("a plain name resolves inside the install dir");
        assert!(ok.starts_with(&j.install_dir));
        for bad in [
            r"..\..\Windows\System32\evil.dll",
            r"C:\Windows\evil.dll",
            r"sub\evil.dll",
            "",
            ".",
        ] {
            assert!(
                j.backup_path(bad).is_none(),
                "backup name {bad:?} must be refused"
            );
        }
    }

    #[test]
    fn confinement_check() {
        let j = sample();
        assert!(j.is_path_confined("C:\\Users\\x\\Programs\\MyApp"));
        assert!(j.is_path_confined("C:\\Users\\x\\Programs\\MyApp\\MyApp.exe"));
        assert!(j.is_path_confined("c:/users/x/programs/myapp/sub/file.txt"));
        assert!(!j.is_path_confined("C:\\Windows\\System32\\evil.exe"));
        assert!(!j.is_path_confined("C:\\Users\\x\\Programs\\MyApp2\\evil.exe"));
        assert!(!j.is_path_confined(""));
    }

    #[test]
    fn bad_journals_are_rejected() {
        let mut j = sample();
        j.schema_version = 99;
        assert!(j.validate().is_err());
        let mut j = sample();
        j.install_dir = String::new();
        assert!(j.validate().is_err());
        assert!(InstallJournal::from_json("{\"schema_version\":99}").is_err());
    }

    #[test]
    fn save_and_load_via_disk() {
        let dir = crate::scratch::ScratchDir::new("rustpacker-journal-test");
        let path = dir.join(JOURNAL_FILE_NAME);
        sample().save(&path).expect("save");
        let back = InstallJournal::load(&path).expect("load");
        assert_eq!(back.app_id, "MyApp");
    }
}
