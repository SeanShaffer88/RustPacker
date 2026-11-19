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

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid install journal: {e}"))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let text = self.to_json()?;
        // Atomic write: temp file in the same folder, then rename.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("could not write journal: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("could not save journal: {e}"))
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
    fn save_and_load_via_disk() {
        let dir =
            std::env::temp_dir().join(format!("rustpacker-journal-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(JOURNAL_FILE_NAME);
        sample().save(&path).expect("save");
        let back = InstallJournal::load(&path).expect("load");
        assert_eq!(back.app_id, "MyApp");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
