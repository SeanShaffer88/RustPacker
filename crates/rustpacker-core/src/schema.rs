//! Project schema for RustPacker packages.
//!
//! A project file is human-readable TOML. Every config carries a
//! `schema_version` and is migrated upward explicitly (see
//! [`ProjectConfig::migrate`]).

use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// One Dark Pro terminal palette (zhuangtongfa.Material-theme).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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
    DualMode,
}

/// Tray behaviour of the installed application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
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
            vec![cfg.tray.tray_arg.clone()]
        } else {
            Vec::new()
        }
    }

    /// The `HKCU\...\Run` value data for launch-at-login, or `None` when
    /// the option is not visible / not chosen.
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
        if start_in_tray && self.show_login_tray {
            Some(format!(
                "\"{exe_path}\" {}",
                cfg.tray.effective_startup_tray_arg()
            ))
        } else {
            Some(format!("\"{exe_path}\""))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutConfig {
    pub name: String,
    /// `desktop`, `start-menu`, or a folder path under the install dir.
    pub location: String,
    pub target: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// `HKCU` or `HKLM`.
    pub root: String,
    pub key: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAssociation {
    pub extension: String,
    pub prog_id: String,
    pub description: String,
    /// Executable (relative to install dir) that opens the file, `%1` appended.
    pub open_with: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct ProcessCheck {
    /// Process image name that must not be running, e.g. `MyApp.exe`.
    pub process_name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

impl ProjectConfig {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let mut cfg: ProjectConfig =
            toml::from_str(text).map_err(|e| format!("invalid project file: {e}"))?;
        cfg.migrate()?;
        Ok(cfg)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("could not serialise project: {e}"))
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut cfg: ProjectConfig =
            serde_json::from_str(text).map_err(|e| format!("invalid project JSON: {e}"))?;
        cfg.migrate()?;
        Ok(cfg)
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
        }
        if self.install.source_dir.trim().is_empty() {
            errs.push("install.source_dir must not be empty".into());
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
        }
        for a in &self.file_associations {
            if !a.extension.starts_with('.') || a.extension.len() < 2 {
                errs.push(format!(
                    "file association extension must look like '.ext': {}",
                    a.extension
                ));
            }
        }
        for r in &self.registry {
            if r.root != "HKCU" && r.root != "HKLM" {
                errs.push(format!("registry root must be HKCU or HKLM: {}", r.root));
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
        let mut cfg = ProjectConfig::default();
        cfg.schema_version = CURRENT_SCHEMA_VERSION + 1;
        assert!(cfg.migrate().is_err());
    }
}
