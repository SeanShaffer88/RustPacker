//! RustPacker core: project schema, terminal theme, typewriter,
//! transactional install journal, and the PE-overlay payload packer.
//!
//! Everything here is platform-independent and fully unit-tested; the
//! Windows wizard (stub), CLI, and Builder Studio are thin surfaces
//! over these operations.

pub mod journal;
pub mod log;
pub mod payload;
pub mod schema;
pub mod scratch;
pub mod typewriter;

pub use journal::{
    InstallJournal, JournalAction, JournalEntry, JOURNAL_FILE_NAME, UNINSTALLER_FILE_NAME,
};
pub use log::{InstallLog, StderrLog, LOG_FILE_NAME};
pub use payload::{
    extract_payload, extract_payload_with_progress, find_payload, normalized_rel_str,
    pack_directory, pack_files, read_manifest, sanitise_rel_path, sha256_hex, sha256sums_txt,
    strip_payload,
    verify_payload, write_stored_zip, write_stored_zip_streaming, ExtractProgress, FileEntry,
    PackedInfo, PayloadManifest, ZipSource, CONFIG_ENTRY_NAME, FOOTER_LEN, MAX_ARCHIVE_LEN,
    MAX_FILE_COUNT, OVERLAY_MAGIC,
};
pub use schema::{
    is_bare_filename, safe_file_name, validate_install_dir, AppInfo, FileAssociation, FinishConfig,
    FinishScreenModel, FinishSelection, InstallConfig, InstallScope, Prerequisite, ProcessCheck, ProjectConfig,
    RegistryEntry, ShortcutConfig, TerminalTheme, TrayConfig, CURRENT_SCHEMA_VERSION,
    MAX_INSTALL_PATH, MAX_SAFE_NAME,
};
pub use scratch::ScratchDir;
pub use typewriter::{Typewriter, DEFAULT_INTERVAL_MS};

/// Library version, kept in step with the workspace version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
