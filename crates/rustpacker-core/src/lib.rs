//! RustPacker core: project schema, terminal theme, typewriter,
//! transactional install journal, and the PE-overlay payload packer.
//!
//! Everything here is platform-independent and fully unit-tested; the
//! Windows wizard (stub), CLI, and Builder Studio are thin surfaces
//! over these operations.

pub mod journal;
pub mod payload;
pub mod schema;
pub mod typewriter;

pub use journal::{
    InstallJournal, JournalAction, JournalEntry, JOURNAL_FILE_NAME, UNINSTALLER_FILE_NAME,
};
pub use payload::{
    extract_payload, find_payload, pack_directory, pack_files, read_manifest, sha256_hex,
    sha256sums_txt, verify_payload, write_stored_zip, FileEntry, PackedInfo, PayloadManifest,
    CONFIG_ENTRY_NAME, OVERLAY_MAGIC,
};
pub use schema::{
    AppInfo, FileAssociation, FinishConfig, FinishScreenModel, InstallConfig, InstallScope,
    Prerequisite, ProcessCheck, ProjectConfig, RegistryEntry, ShortcutConfig, TerminalTheme,
    TrayConfig, CURRENT_SCHEMA_VERSION,
};
pub use typewriter::{Typewriter, DEFAULT_INTERVAL_MS};

/// Library version, kept in step with the workspace version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
