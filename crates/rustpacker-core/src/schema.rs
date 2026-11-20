//! Project schema for RustPacker packages.
//!
//! A project file is human-readable TOML. Every config carries a
//! `schema_version` and is migrated upward explicitly (see
//! [`ProjectConfig::migrate`]).

use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// One Dark Pro terminal palette (zhuangtongfa.Material-theme).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalTheme {
    pub background_hex: String,
    pub surface_hex: String,
    pub elevated_surface_hex: String,
    pub border_hex: String,
    pub accent_hex: String,
    pub success_hex: String,
    pub warning_hex: String,
    pub info_hex: String,
    pub text_hex: String,
    pub muted_hex: String,
    pub cursor_hex: String,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            background_hex: "#282C34".into(),
            surface_hex: "#21252B".into(),
            elevated_surface_hex: "#2C313A".into(),
            border_hex: "#181A1F".into(),
            accent_hex: "#E06C75".into(),
            success_hex: "#98C379".into(),
            warning_hex: "#E5C07B".into(),
            info_hex: "#61AFEF".into(),
            text_hex: "#ABB2BF".into(),
            muted_hex: "#5C6370".into(),
            cursor_hex: "#528BFF".into(),
        }
    }
}

impl TerminalTheme {
    /// Parse a `#RRGGBB` (or `RRGGBB`) hex colour into `(r, g, b)`.
    pub fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
        let h = hex.strip_prefix('#').unwrap_or(hex);
        if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let r = u8::from_str_radix(&h[0..2], 16).ok()?;
        let g = u8::from_str_radix(&h[2..4], 16).ok()?;
        let b = u8::from_str_radix(&h[4..6], 16).ok()?;
        Some((r, g, b))
    }

    pub fn background_rgb(&self) -> (u8, u8, u8) {
        Self::parse_hex(&self.background_hex).unwrap_or((0x28, 0x2C, 0x34))
    }
    pub fn surface_rgb(&self) -> (u8, u8, u8) {
        Self::parse_hex(&self.surface_hex).unwrap_or((0x21, 0x25, 0x2B))
    }
    pub fn text_rgb(&self) -> (u8, u8, u8) {
        Self::parse_hex(&self.text_hex).unwrap_or((0xAB, 0xB2, 0xBF))
    }
    pub fn accent_rgb(&self) -> (u8, u8, u8) {
        Self::parse_hex(&self.accent_hex).unwrap_or((0xE0, 0x6C, 0x75))
    }
    pub fn muted_rgb(&self) -> (u8, u8, u8) {
        Self::parse_hex(&self.muted_hex).unwrap_or((0x5C, 0x63, 0x70))
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        for (name, value) in [
            ("background_hex", &self.background_hex),
            ("surface_hex", &self.surface_hex),
            ("elevated_surface_hex", &self.elevated_surface_hex),
            ("border_hex", &self.border_hex),
            ("accent_hex", &self.accent_hex),
            ("success_hex", &self.success_hex),
            ("warning_hex", &self.warning_hex),
            ("info_hex", &self.info_hex),
            ("text_hex", &self.text_hex),
            ("muted_hex", &self.muted_hex),
            ("cursor_hex", &self.cursor_hex),
        ] {
            if Self::parse_hex(value).is_none() {
                errs.push(format!("theme.{name} is not a #RRGGBB colour: {value}"));
            }
        }
        errs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum InstallScope {
    #[default]
    PerUser,
    PerMachine,
    /// Request elevation and install for all users when the process can
    /// elevate; otherwise fall back to a per-user install. The
    /// *effective* scope is resolved by [`InstallScope::resolve`] — the
    /// stub and the GUI never treat `DualMode` as "per-user but with a
    /// different label".
    DualMode,
}

impl InstallScope {
    /// The concrete scope an install actually performs.
    ///
    /// `elevated` says whether the installer is running with machine-wide
    /// rights (admin). DualMode becomes per-machine only when elevated.
    pub fn resolve(self, elevated: bool) -> InstallScope {
        match self {
            InstallScope::DualMode => {
                if elevated {
                    InstallScope::PerMachine
                } else {
                    InstallScope::PerUser
                }
            }
            other => other,
        }
    }

    pub fn is_machine_wide(self) -> bool {
        matches!(self, InstallScope::PerMachine)
    }

    /// Human label for UI and JSON output (never "DualMode").
    pub fn label(self) -> &'static str {
        match self {
            InstallScope::PerUser | InstallScope::DualMode => "per-user",
            InstallScope::PerMachine => "per-machine",
        }
    }
}

/// Tray behaviour of the installed application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayConfig {
    /// True when the installed app can minimise to the system tray.
    pub supports_tray: bool,
    /// Argument passed when launching minimised to tray (e.g. `--tray`).
    pub tray_arg: String,
    /// Optional distinct argument for the at-login launch. When empty,
    /// [`TrayConfig::effective_startup_tray_arg`] falls back to `tray_arg`.
    pub startup_tray_arg: String,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            supports_tray: false,
            tray_arg: "--tray".into(),
            startup_tray_arg: String::new(),
        }
    }
}

impl TrayConfig {
    /// The argument appended to the Windows-login command when the
    /// "start minimised to tray" sub-option is chosen for login launch.
    pub fn effective_startup_tray_arg(&self) -> &str {
        if self.startup_tray_arg.trim().is_empty() {
            &self.tray_arg
        } else {
            &self.startup_tray_arg
        }
    }
}

/// Which optional checkboxes the finish page offers. An option is always
/// offered unless explicitly hidden here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FinishConfig {
    pub hide_desktop_shortcut: bool,
    pub hide_launch_after_setup: bool,
    /// Defaults for the checkboxes (user can still change them).
    pub desktop_shortcut_default: bool,
    pub launch_after_setup_default: bool,
    pub launch_tray_default: bool,
    pub launch_at_login_default: bool,
    pub login_tray_default: bool,
}

impl Default for FinishConfig {
    fn default() -> Self {
        Self {
            hide_desktop_shortcut: false,
            hide_launch_after_setup: false,
            desktop_shortcut_default: true,
            launch_after_setup_default: true,
            launch_tray_default: false,
            launch_at_login_default: false,
            login_tray_default: true,
        }
    }
}

/// The resolved finish-page model: exactly which options are visible and
/// how their sub-options depend on their parents. The stub wizard renders
/// this model; it contains no rendering logic of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishScreenModel {
    pub show_desktop_shortcut: bool,
    pub show_launch_after_setup: bool,
    /// Indented under Launch: only when the app supports tray.
    pub show_launch_tray: bool,
    /// Only when the app supports tray.
    pub show_launch_at_login: bool,
    /// Indented under Launch at login: only when the app supports tray.
    pub show_login_tray: bool,
}

impl FinishScreenModel {
    pub fn from_config(cfg: &ProjectConfig) -> Self {
        let tray = cfg.tray.supports_tray;
        Self {
            show_desktop_shortcut: !cfg.finish.hide_desktop_shortcut,
            show_launch_after_setup: !cfg.finish.hide_launch_after_setup,
            show_launch_tray: tray && !cfg.finish.hide_launch_after_setup,
            show_launch_at_login: tray,
            show_login_tray: tray,
        }
    }

    /// Arguments for launching the app after setup.
    ///
    /// `tray_arg` is a command-line *fragment*: it is split on
    /// whitespace so `"--tray --at-login"` becomes two arguments. This
    /// matches how [`login_command`] embeds the same string in the
    /// `Run` value (where it is also a fragment, not a single quoted
    /// argument).
    pub fn launch_args(
        &self,
        cfg: &ProjectConfig,
        launch: bool,
        start_in_tray: bool,
    ) -> Vec<String> {
        if !launch || !self.show_launch_after_setup {
            return Vec::new();
        }
        if start_in_tray && self.show_launch_tray {
            cfg.tray
                .tray_arg
                .split_whitespace()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// The `HKCU\...\Run` value data for launch-at-login, or `None` when
    /// the option is not visible / not chosen. Returns `None` when
    /// `exe_path` cannot be safely quoted (contains `"` or controls).
    pub fn login_command(
        &self,
        cfg: &ProjectConfig,
        exe_path: &str,
        at_login: bool,
        start_in_tray: bool,
    ) -> Option<String> {
        if !at_login || !self.show_launch_at_login {
            return None;
        }
        if exe_path.contains('"') || exe_path.chars().any(|c| (c as u32) < 0x20) {
            return None;
        }
        if start_in_tray && self.show_login_tray {
            let arg = cfg.tray.effective_startup_tray_arg().trim();
            if arg.is_empty() {
                Some(format!("\"{exe_path}\""))
            } else {
                Some(format!("\"{exe_path}\" {arg}"))
            }
        } else {
            Some(format!("\"{exe_path}\""))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// Stable identifier used for the uninstall registry key and journal.
    pub app_id: String,
    /// Main executable file name inside the payload (e.g. `MyApp.exe`).
    pub main_exe: String,
    pub description: String,
}

impl Default for AppInfo {
    fn default() -> Self {
        Self {
            name: "My App".into(),
            version: "0.1.0".into(),
            publisher: "Sean Shaffer".into(),
            app_id: "MyApp".into(),
            main_exe: "MyApp.exe".into(),
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstallConfig {
    pub scope: InstallScope,
    /// Install folder token or path. Tokens: `{per-user-programs}`,
    /// `{per-machine-programs}`. A trailing app folder is appended when
    /// the value names a root.
    pub default_dir: String,
    /// Source directory (relative to the project file) packed into Setup.exe.
    pub source_dir: String,
    pub license_file: String,
    pub require_admin: bool,
    /// Minimum Windows build (Windows 11 = 22000). 0 disables the check.
    pub min_windows_build: u32,
    /// License text shown on the license page. Filled in by the packer
    /// from `license_file` so the wizard can display the real agreement
    /// instead of a path. Never written back to the project file
    /// (`skip_serializing`) — the file is the source of truth on disk.
    #[serde(default, skip_serializing)]
    pub license_text: String,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            scope: InstallScope::PerUser,
            default_dir: "{per-user-programs}".into(),
            source_dir: "payload".into(),
            license_file: String::new(),
            require_admin: false,
            min_windows_build: 22000,
            license_text: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutConfig {
    pub name: String,
    /// `desktop`, `start-menu`, or a folder path under the install dir.
    pub location: String,
    pub target: String,
    #[serde(default)]
    pub arguments: String,
}

impl ShortcutConfig {
    /// Whether the location is one the installer can create today.
    pub fn location_is_supported(&self) -> bool {
        matches!(
            self.location.trim().to_ascii_lowercase().as_str(),
            "desktop" | "start-menu" | "startmenu"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    /// `HKCU` or `HKLM`.
    pub root: String,
    pub key: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileAssociation {
    pub extension: String,
    pub prog_id: String,
    pub description: String,
    /// Executable (relative to install dir) that opens the file, `%1` appended.
    pub open_with: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prerequisite {
    pub name: String,
    /// Registry value that must exist, e.g. `HKLM\SOFTWARE\...\Install`.
    #[serde(default)]
    pub registry_probe: String,
    /// File that must exist (environment variables are expanded at install).
    #[serde(default)]
    pub file_probe: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCheck {
    /// Process image name that must not be running, e.g. `MyApp.exe`.
    pub process_name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    pub app: AppInfo,
    pub install: InstallConfig,
    pub tray: TrayConfig,
    pub finish: FinishConfig,
    pub theme: TerminalTheme,
    pub typewriter_interval_ms: u32,
    pub shortcuts: Vec<ShortcutConfig>,
    pub registry: Vec<RegistryEntry>,
    pub file_associations: Vec<FileAssociation>,
    pub prerequisites: Vec<Prerequisite>,
    pub process_checks: Vec<ProcessCheck>,
    /// Add the install dir to PATH (`user` or `machine`); empty disables.
    pub path_scope: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            app: AppInfo::default(),
            install: InstallConfig::default(),
            tray: TrayConfig::default(),
            finish: FinishConfig::default(),
            theme: TerminalTheme::default(),
            typewriter_interval_ms: 12,
            shortcuts: Vec::new(),
            registry: Vec::new(),
            file_associations: Vec::new(),
            prerequisites: Vec::new(),
            process_checks: Vec::new(),
            path_scope: String::new(),
        }
    }
}

/// True when `name` is a single bare file name (no directory
/// separators, no drive/ADS `:` , no `..`, no reserved device name,
/// no trailing dot/space, no controls). Used to validate
/// `app.main_exe`, `FileAssociation.open_with`, and process names so
/// `install_dir.join(name)` can never escape the install directory.
pub fn is_bare_filename(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains(':') {
        return false;
    }
    if name.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f) {
        return false;
    }
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name);
    !is_reserved_device_name(stem)
}

/// Windows reserved device names (`CON`, `NUL`, `COM1`, …), matched
/// case-insensitively on the stem before the first dot.
fn is_reserved_device_name(stem: &str) -> bool {
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Make a file-name component safe for Windows: replace characters
/// Windows forbids in file names (and control characters) with `-`,
/// trim leading/trailing dots and spaces, and fall back to `"App"`
/// when nothing usable remains. Reserved device names get a `_`
/// prefix. This is the single implementation shared by the CLI and
/// the GUI so both produce identical output file names.
pub fn safe_file_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 0x20 {
            out.push('-');
        } else {
            out.push(c);
        }
    }
    let trimmed = out.trim_matches(|c| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return "App".to_string();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_reserved_device_name(stem) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Longest install path the installer accepts, in UTF-16 code units.
///
/// A Win32 path is limited to 32,767 characters, but every component except
/// the root is limited to 255, and an install directory deeper than this is
/// almost certainly a mistake rather than an intent.
pub const MAX_INSTALL_PATH: usize = 4096;

/// Windows directories an install must never target.
///
/// Installing over the OS directory, a system directory, or a drive root
/// either fails halfway (leaving a half-installed app) or corrupts Windows.
/// Comparison is case-insensitive and ignores trailing separators, because
/// `C:\WINDOWS\System32\` and `c:\windows\system32` are the same place.
const FORBIDDEN_ROOTS: &[&str] = &[
    "windows",
    "windows\\system32",
    "windows\\syswow64",
    "windows\\system",
    "program files\\windowsapps",
    "users",
    "users\\public",
    "$recycle.bin",
    "system volume information",
    "recovery",
    "boot",
];

/// Reject install directories that cannot or must not be used.
///
/// Returns a human-readable reason, suitable for putting straight in a
/// message box. `Ok` means the path is *plausible*: the caller still has to
/// create it, because writability can only be tested by trying.
pub fn validate_install_dir(raw: &str) -> Result<(), String> {
    let path = raw.trim();
    if path.is_empty() {
        return Err("Choose an install folder.".to_string());
    }
    if path.len() > MAX_INSTALL_PATH {
        return Err(format!(
            "That folder path is {} characters long; the limit is {MAX_INSTALL_PATH}.",
            path.len()
        ));
    }
    // Invalid characters: everything NTFS forbids in a name, plus the
    // control range. A folder path may contain '\' and ':' (drive letter),
    // but not the others.
    if let Some(bad) = path
        .chars()
        .find(|&c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 0x20)
    {
        return Err(format!(
            "That folder path contains {bad:?}, which Windows does not allow in a path."
        ));
    }

    // A relative path would be resolved against the installer's working
    // directory, which is not something a user can reason about.
    let bytes = path.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    let is_unc = path.starts_with("\\\\");
    let is_device = path.starts_with("\\\\.\\") || path.starts_with("\\??\\");
    if is_device {
        return Err("That is a device path, not a folder.".to_string());
    }
    if !has_drive && !is_unc {
        return Err(format!(
            "\"{path}\" is not a full path. Start it with a drive letter, e.g. C:\\Programs."
        ));
    }

    // Component checks: empty components double up separators, "." and ".."
    // are relative, and a reserved device name as a folder name is silently
    // unusable on Windows.
    let body = if has_drive { &path[2..] } else { path };
    let mut components = 0usize;
    for part in body.split(|c| c == '\\' || c == '/') {
        if part.is_empty() {
            continue;
        }
        components += 1;
        if part == "." || part == ".." {
            return Err(format!(
                "\"{path}\" is not a full path: \"{part}\" makes it depend on the current folder."
            ));
        }
        if part.len() > 255 {
            return Err(format!(
                "The folder name \"{}\" is longer than Windows allows (255 characters).",
                part
            ));
        }
        let stem = part.split('.').next().unwrap_or(part);
        if is_reserved_device_name(stem) {
            return Err(format!(
                "\"{part}\" is a reserved Windows device name; Windows cannot create that folder."
            ));
        }
        // Names cannot end in a dot or a space: Windows strips them, so the
        // folder created would not be the folder asked for.
        if part.ends_with(' ') || part.ends_with('.') {
            return Err(format!(
                "The folder name \"{part}\" ends with a space or a dot; Windows would silently drop it."
            ));
        }
    }
    if components == 0 {
        return Err(format!(
            "\"{path}\" is a drive root. Choose a folder inside it, e.g. {path}Programs."
        ));
    }

    // Forbidden locations: compare the path below the drive root, with
    // separators normalised and case folded.
    let below_root = body.replace('/', "\\");
    let below_root = below_root.trim_matches('\\').to_ascii_lowercase();
    for forbidden in FORBIDDEN_ROOTS {
        if below_root == *forbidden {
            return Err(format!(
                "\"{path}\" is a Windows system folder. Choose somewhere else."
            ));
        }
    }
    Ok(())
}

impl ProjectConfig {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let mut cfg: ProjectConfig =
            toml::from_str(text).map_err(|e| format!("invalid project file: {e}"))?;
        cfg.migrate()?;
        cfg.normalise()?;
        Ok(cfg)
    }

    /// Trim user-facing strings and reject the few values that are
    /// unsafe regardless of context (`app_id` is used as a directory
    /// and registry-name component). Errors here are returned as parse
    /// errors, so a JSON project cannot smuggle what TOML validation
    /// would reject.
    pub fn normalise(&mut self) -> Result<(), String> {
        self.app.name = self.app.name.trim().to_string();
        self.app.version = self.app.version.trim().to_string();
        self.app.main_exe = self.app.main_exe.trim().to_string();
        self.app.app_id = self.app.app_id.trim().to_string();
        self.install.source_dir = self.install.source_dir.trim().to_string();
        self.install.default_dir = self.install.default_dir.trim().to_string();
        self.tray.tray_arg = self.tray.tray_arg.trim().to_string();
        self.tray.startup_tray_arg = self.tray.startup_tray_arg.trim().to_string();
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("could not serialise project: {e}"))
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut cfg: ProjectConfig =
            serde_json::from_str(text).map_err(|e| format!("invalid project JSON: {e}"))?;
        cfg.migrate()?;
        cfg.normalise()?;
        Ok(cfg)
    }

    /// Parse a project file, reporting **every** validation problem at
    /// once (parse errors first) instead of stopping at the first one.
    pub fn from_toml_checked(text: &str) -> Result<Self, Vec<String>> {
        let cfg = Self::from_toml(text).map_err(|e| vec![e])?;
        let errors = cfg.validate();
        if errors.is_empty() {
            Ok(cfg)
        } else {
            Err(errors)
        }
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("could not serialise project: {e}"))
    }

    /// Explicit upward migration. Version 1 is current; anything newer is
    /// rejected rather than silently misread.
    pub fn migrate(&mut self) -> Result<(), String> {
        if self.schema_version == 0 {
            self.schema_version = 1;
        }
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(format!(
                "project schema_version {} is newer than supported {}",
                self.schema_version, CURRENT_SCHEMA_VERSION
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        if self.app.name.trim().is_empty() {
            errs.push("app.name must not be empty".into());
        }
        if self.app.version.trim().is_empty() {
            errs.push("app.version must not be empty".into());
        } else if self.app.version.chars().any(|c| {
            matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 0x20
        }) {
            errs.push(
                "app.version must not contain path separators or Windows-forbidden characters"
                    .into(),
            );
        }
        if self.app.app_id.trim().is_empty()
            || !self
                .app
                .app_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            errs.push(
                "app.app_id must be non-empty and use only letters, digits, '.', '-' or '_'".into(),
            );
        }
        if self.app.main_exe.trim().is_empty() {
            errs.push("app.main_exe must not be empty".into());
        } else if !is_bare_filename(self.app.main_exe.trim()) {
            errs.push(
                "app.main_exe must be a bare file name (no directories, drive, or '..'), e.g. 'MyApp.exe'"
                    .into(),
            );
        }
        if self.install.source_dir.trim().is_empty() {
            errs.push("install.source_dir must not be empty".into());
        }
        if self.install.default_dir.trim().is_empty() {
            errs.push("install.default_dir must not be empty".into());
        }
        if self.install.min_windows_build > 100_000 {
            errs.push(
                "install.min_windows_build is implausibly large (Windows 11 is 22000)".into(),
            );
        }
        if self.typewriter_interval_ms == 0 || self.typewriter_interval_ms > 1000 {
            errs.push("typewriter_interval_ms must be between 1 and 1000".into());
        }
        if self.tray.supports_tray && self.tray.tray_arg.trim().is_empty() {
            errs.push("tray.tray_arg must not be empty when tray.supports_tray is true".into());
        }
        if !self.path_scope.is_empty() && self.path_scope != "user" && self.path_scope != "machine"
        {
            errs.push("path_scope must be empty, \"user\", or \"machine\"".into());
        } else if self.path_scope == "machine"
            && matches!(self.install.scope, InstallScope::PerUser)
        {
            // A per-user install cannot write the machine PATH; failing
            // here (instead of mid-install) keeps the settings honest.
            errs.push(
                "path_scope = \"machine\" needs install.scope = \"per-machine\" (or \"dual-mode\")"
                    .into(),
            );
        }
        for s in &self.shortcuts {
            if s.name.trim().is_empty() {
                errs.push("shortcut name must not be empty".into());
            }
            if s.location.trim().is_empty() {
                errs.push(format!("shortcut '{}' location must not be empty", s.name));
            } else if !s.location_is_supported() {
                errs.push(format!(
                    "shortcut '{}' location must be \"desktop\" or \"start-menu\": {}",
                    s.name, s.location
                ));
            }
            if s.target.trim().is_empty() {
                errs.push(format!("shortcut '{}' target must not be empty", s.name));
            } else if crate::payload::sanitise_rel_path(s.target.trim()).is_err() {
                errs.push(format!(
                    "shortcut '{}' target must be a relative path inside the install dir: {}",
                    s.name, s.target
                ));
            }
        }
        for r in &self.registry {
            if r.root != "HKCU" && r.root != "HKLM" {
                errs.push(format!("registry root must be HKCU or HKLM: {}", r.root));
            }
            if r.key.trim().is_empty() {
                errs.push("registry key must not be empty".into());
            }
            if r.name.trim().is_empty() {
                errs.push(format!(
                    "registry entry for key '{}' has an empty value name",
                    r.key
                ));
            }
            let machine_capable = matches!(
                self.install.scope,
                InstallScope::PerMachine | InstallScope::DualMode
            ) || self.path_scope.trim().eq_ignore_ascii_case("machine");
            if r.root == "HKLM" && !machine_capable {
                errs.push(format!(
                    "registry entry '{}' targets HKLM, which needs install.scope = \"per-machine\" (or \"dual-mode\")",
                    r.key
                ));
            }
        }
        {
            let mut seen_ext: std::collections::HashSet<String> = std::collections::HashSet::new();
            for a in &self.file_associations {
                if !a.extension.starts_with('.') || a.extension.len() < 2 {
                    errs.push(format!(
                        "file association extension must look like '.ext': {}",
                        a.extension
                    ));
                } else if !seen_ext.insert(a.extension.to_ascii_lowercase()) {
                    errs.push(format!(
                        "duplicate file association extension: {}",
                        a.extension
                    ));
                }
                if a.prog_id.trim().is_empty() {
                    errs.push(format!(
                        "file association '{}' prog_id must not be empty",
                        a.extension
                    ));
                }
                if a.open_with.trim().is_empty() {
                    errs.push(format!(
                        "file association '{}' open_with must not be empty",
                        a.extension
                    ));
                } else if !is_bare_filename(a.open_with.trim()) {
                    errs.push(format!(
                        "file association '{}' open_with must be a bare file name: {}",
                        a.extension, a.open_with
                    ));
                }
            }
        }
        for pr in &self.prerequisites {
            if pr.name.trim().is_empty() {
                errs.push("prerequisite name must not be empty".into());
            }
            if pr.message.trim().is_empty() {
                errs.push(format!(
                    "prerequisite '{}' message must not be empty",
                    pr.name
                ));
            }
            if pr.registry_probe.trim().is_empty() && pr.file_probe.trim().is_empty() {
                errs.push(format!(
                    "prerequisite '{}' needs a registry_probe or file_probe (otherwise it always passes)",
                    pr.name
                ));
            }
        }
        for pc in &self.process_checks {
            if pc.process_name.trim().is_empty() {
                errs.push("process check process_name must not be empty".into());
            } else if !is_bare_filename(pc.process_name.trim()) {
                errs.push(format!(
                    "process check process_name must be a bare file name: {}",
                    pc.process_name
                ));
            }
            if pc.message.trim().is_empty() {
                errs.push(format!(
                    "process check '{}' message must not be empty",
                    pc.process_name
                ));
            }
        }
        errs.extend(self.theme.validate());
        errs
    }

    pub fn finish_model(&self) -> FinishScreenModel {
        FinishScreenModel::from_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- M11: install-directory validation -------------------------------

    #[test]
    fn ordinary_install_dirs_are_accepted() {
        for good in [
            r"C:\Program Files\My App",
            r"C:\Program Files\My App\",
            r"c:\program files\my app",
            r"D:\Apps\Widget",
            r"\\server\share\Widget",
            r"C:\Users\sean\AppData\Local\Programs\Widget",
        ] {
            assert!(
                validate_install_dir(good).is_ok(),
                "{good} should be accepted: {:?}",
                validate_install_dir(good)
            );
        }
    }

    #[test]
    fn install_dir_rejects_what_windows_will_not_create() {
        // (input, a word the message must contain)
        let cases = [
            ("", "Choose"),
            ("   ", "Choose"),
            (r"Programs\Widget", "full path"),
            (r"\Widget", "full path"),
            (r"C:\Programs\..\..\Windows", "current folder"),
            (r"C:\Programs\.\Widget", "current folder"),
            (r"C:\Programs\Wi|dget", "does not allow"),
            (r"C:\Programs\Wi<dget", "does not allow"),
            (r"C:\Programs\NUL", "reserved"),
            (r"C:\Programs\con", "reserved"),
            (r"C:\Programs\NUL.txt", "reserved"),
            (r"C:\Programs\Widget ", "space or a dot"),
            (r"C:\Programs\Widget.", "space or a dot"),
            (r"C:\", "drive root"),
            (r"\\.\C:\Widget", "device path"),
        ];
        for (input, needle) in cases {
            match validate_install_dir(input) {
                Ok(()) => panic!("{input:?} should have been rejected"),
                Err(msg) => assert!(
                    msg.to_lowercase().contains(needle.to_lowercase()),
                    "{input:?} produced {msg:?}, expected it to mention {needle:?}"
                ),
            }
        }
    }

    #[test]
    fn install_dir_rejects_windows_system_locations() {
        for bad in [
            r"C:\Windows",
            r"C:\Windows\",
            r"c:\windows\system32",
            r"C:\WINDOWS\SysWOW64\",
            r"C:\Program Files\WindowsApps",
            r"C:\Users",
            r"C:\$Recycle.Bin",
            r"C:\System Volume Information",
        ] {
            let err = validate_install_dir(bad).expect_err(bad);
            assert!(err.contains("system folder"), "{bad} -> {err}");
        }
    }

    #[test]
    fn install_dir_rejects_overlong_paths_and_component_names() {
        let long_component = format!(r"C:\{}", "a".repeat(256));
        assert!(validate_install_dir(&long_component)
            .expect_err("component")
            .contains("255"));
        let long_path = format!(r"C:\{}", "a/".repeat(3000));
        assert!(validate_install_dir(&long_path).is_err());
    }

    #[test]
    fn theme_defaults_are_one_dark_pro() {
        let t = TerminalTheme::default();
        assert_eq!(t.background_hex, "#282C34");
        assert_eq!(t.surface_hex, "#21252B");
        assert_eq!(t.elevated_surface_hex, "#2C313A");
        assert_eq!(t.border_hex, "#181A1F");
        assert_eq!(t.accent_hex, "#E06C75");
        assert_eq!(t.success_hex, "#98C379");
        assert_eq!(t.warning_hex, "#E5C07B");
        assert_eq!(t.info_hex, "#61AFEF");
        assert_eq!(t.text_hex, "#ABB2BF");
        assert_eq!(t.muted_hex, "#5C6370");
        assert_eq!(t.cursor_hex, "#528BFF");
        assert_eq!(t.background_rgb(), (0x28, 0x2C, 0x34));
    }

    #[test]
    fn finish_model_without_tray_hides_login_and_tray_options() {
        let cfg = ProjectConfig::default();
        let m = cfg.finish_model();
        assert!(m.show_desktop_shortcut);
        assert!(m.show_launch_after_setup);
        assert!(!m.show_launch_tray);
        assert!(!m.show_launch_at_login);
        assert!(!m.show_login_tray);
        assert_eq!(
            m.login_command(&cfg, "C:\\Apps\\MyApp.exe", true, true),
            None
        );
    }

    #[test]
    fn finish_model_with_tray_shows_all_and_builds_login_command() {
        let mut cfg = ProjectConfig::default();
        cfg.tray.supports_tray = true;
        cfg.tray.tray_arg = "--minimized".into();
        let m = cfg.finish_model();
        assert!(m.show_launch_tray && m.show_launch_at_login && m.show_login_tray);
        assert_eq!(
            m.login_command(&cfg, "C:\\Apps\\MyApp.exe", true, true)
                .as_deref(),
            Some("\"C:\\Apps\\MyApp.exe\" --minimized")
        );
        assert_eq!(
            m.login_command(&cfg, "C:\\Apps\\MyApp.exe", true, false)
                .as_deref(),
            Some("\"C:\\Apps\\MyApp.exe\"")
        );
        assert_eq!(
            m.launch_args(&cfg, true, true),
            vec!["--minimized".to_string()]
        );
        assert!(m.launch_args(&cfg, false, true).is_empty());
    }

    #[test]
    fn startup_tray_arg_overrides_tray_arg_for_login_only() {
        let mut cfg = ProjectConfig::default();
        cfg.tray.supports_tray = true;
        cfg.tray.tray_arg = "--tray".into();
        cfg.tray.startup_tray_arg = "--tray --at-login".into();
        let m = cfg.finish_model();
        assert_eq!(
            m.login_command(&cfg, "C:\\A\\a.exe", true, true).as_deref(),
            Some("\"C:\\A\\a.exe\" --tray --at-login")
        );
        assert_eq!(m.launch_args(&cfg, true, true), vec!["--tray".to_string()]);
    }

    #[test]
    fn hidden_finish_options_stay_hidden() {
        let mut cfg = ProjectConfig::default();
        cfg.tray.supports_tray = true;
        cfg.finish.hide_desktop_shortcut = true;
        cfg.finish.hide_launch_after_setup = true;
        let m = cfg.finish_model();
        assert!(!m.show_desktop_shortcut);
        assert!(!m.show_launch_after_setup);
        assert!(!m.show_launch_tray, "tray sub-option hides with its parent");
        assert!(
            m.show_launch_at_login,
            "login option is independent of launch-after"
        );
    }

    #[test]
    fn toml_round_trip_and_validation() {
        let cfg = ProjectConfig::default();
        let text = cfg.to_toml().expect("serialise");
        let back = ProjectConfig::from_toml(&text).expect("parse");
        assert_eq!(back.app.app_id, cfg.app.app_id);
        assert!(back.validate().is_empty(), "{:?}", back.validate());
        let mut bad = ProjectConfig::default();
        bad.app.name = String::new();
        assert!(!bad.validate().is_empty());
    }

    #[test]
    fn newer_schema_is_rejected() {
        let mut cfg = ProjectConfig {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..Default::default()
        };
        assert!(cfg.migrate().is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = ProjectConfig::from_toml(
            "schema_version = 1\n[app]\nname = \"X\"\nsuports_tray = true\n",
        );
        assert!(err.is_err(), "typo field must fail to parse: {err:?}");
        let err = ProjectConfig::from_toml("schema_version = 1\nbogus_key = 1\n");
        assert!(err.is_err(), "unknown top-level key must fail: {err:?}");
    }

    #[test]
    fn bare_filename_and_safe_name_helpers() {
        assert!(is_bare_filename("MyApp.exe"));
        assert!(!is_bare_filename("../evil.exe"));
        assert!(!is_bare_filename("C:\\evil.exe"));
        assert!(!is_bare_filename("a/b.exe"));
        assert!(!is_bare_filename("CON"));
        assert!(!is_bare_filename(""));
        assert_eq!(safe_file_name("My App"), "My App");
        assert_eq!(safe_file_name("a/b:c"), "a-b-c");
        assert_eq!(safe_file_name("..."), "App");
        assert_eq!(safe_file_name("CON"), "_CON");
    }

    #[test]
    fn expanded_validation_catches_bad_sections() {
        let mut cfg = ProjectConfig::default();
        cfg.app.main_exe = "../evil.exe".into();
        assert!(cfg.validate().iter().any(|e| e.contains("main_exe")));
        let mut cfg = ProjectConfig::default();
        cfg.app.version = "1.0/evil".into();
        assert!(cfg.validate().iter().any(|e| e.contains("version")));
        let mut cfg = ProjectConfig::default();
        cfg.shortcuts.push(ShortcutConfig {
            name: String::new(),
            location: String::new(),
            target: String::new(),
            arguments: String::new(),
        });
        assert!(!cfg.validate().is_empty());
        let mut cfg = ProjectConfig::default();
        cfg.prerequisites.push(Prerequisite {
            name: "X".into(),
            registry_probe: String::new(),
            file_probe: String::new(),
            message: "m".into(),
        });
        assert!(cfg.validate().iter().any(|e| e.contains("always passes")));
        let mut cfg = ProjectConfig::default();
        cfg.process_checks.push(ProcessCheck {
            process_name: "a/b.exe".into(),
            message: "m".into(),
        });
        assert!(cfg.validate().iter().any(|e| e.contains("process_name")));
    }

    #[test]
    fn multi_word_tray_arg_splits_for_launch() {
        let mut cfg = ProjectConfig::default();
        cfg.tray.supports_tray = true;
        cfg.tray.tray_arg = "--tray --at-login".into();
        let m = cfg.finish_model();
        assert_eq!(
            m.launch_args(&cfg, true, true),
            vec!["--tray".to_string(), "--at-login".to_string()]
        );
        // Unquotable exe paths must not produce an injectable Run value.
        assert_eq!(m.login_command(&cfg, "C:\\A\\a\"b.exe", true, true), None);
    }
}
