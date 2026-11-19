//! RustPacker generated-installer stub.
//!
//! When the packer appends a payload to this binary it becomes a
//! self-contained `Setup.exe`: a compact, terminal-styled Win32 wizard
//! (no sidebar graphic, no outer scrollbars, Per-Monitor DPI V2) that
//! extracts the payload, journals every change, copies itself to
//! `Uninstall.exe`, and registers an Apps & Features entry.
//!
//! Modes (also see `--help`):
//!   * (no arguments)          run the install wizard
//!   * `--uninstall`           uninstall using the journal beside this exe
//!   * `--uninstall --silent`  uninstall with no UI at all
//!   * `--help`                show usage in a message box
//! Uninstall mode is also selected automatically when this exe's file
//! name is `Uninstall.exe`.
//!
//! Without a payload the wizard runs in demo mode with a built-in
//! default project ("RustPacker Demo") so the bare stub stays testable.
//!
//! All Windows code lives in `imp` behind `cfg(windows)`; on other
//! hosts the binary just prints an explanation and exits non-zero, so
//! `cargo check` on a Linux/macOS dev box still passes.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "rustpacker-stub is a Windows-only installer stub (Windows 11 x64). \
         This build is running on a non-Windows host, so there is nothing to do."
    );
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    std::process::exit(imp::run());
}

#[cfg(windows)]
mod imp {
    use rustpacker_core::{
        extract_payload, find_payload, read_manifest, sha256_hex, InstallJournal, InstallScope,
        JournalAction, Prerequisite, ProjectConfig, Typewriter, JOURNAL_FILE_NAME,
        UNINSTALLER_FILE_NAME,
    };
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use windows_sys::core::{GUID, HRESULT, PCWSTR};
    use windows_sys::Win32::Foundation::{
        COLORREF, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        CreateFontW, CreateSolidBrush, DeleteObject, SetBkColor, SetTextColor, UpdateWindow,
        HBRUSH, HFONT,
    };
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_SET_VALUE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    use windows_sys::Win32::UI::Controls::{
        ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, PBM_SETPOS, PBM_SETRANGE32,
    };
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        EnableWindow, SetFocus, VK_RETURN, VK_SPACE,
    };
    use windows_sys::Win32::UI::Shell::{
        SHGetFolderPathW, ShellExecuteW, CSIDL_DESKTOPDIRECTORY, CSIDL_PROGRAMS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetWindowTextW, KillTimer, LoadCursorW, MessageBoxW, PostQuitMessage, RegisterClassExW,
        SendMessageW, SetTimer, SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK,
        BM_SETCHECK, BN_CLICKED, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HMENU, IDC_ARROW, IDYES,
        MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, MSG,
        STN_CLICKED, SW_SHOWNORMAL, WM_COMMAND,
        WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN,
        WM_PARENTNOTIFY, WM_SETFONT, WM_TIMER, WNDCLASSEXW,
    };

    // Control IDs.
    const ID_TITLE: i32 = 100;
    const ID_BODY: i32 = 101;
    const ID_DIR_EDIT: i32 = 200;
    const ID_PROGRESS: i32 = 210;
    const ID_STATUS: i32 = 211;
    const ID_CHK_DESKTOP: i32 = 300;
    const ID_CHK_LAUNCH: i32 = 301;
    const ID_CHK_LAUNCH_TRAY: i32 = 302;
    const ID_CHK_LOGIN: i32 = 303;
    const ID_CHK_LOGIN_TRAY: i32 = 304;
    const ID_BACK: i32 = 9001;
    const ID_NEXT: i32 = 9002;
    const ID_CANCEL: i32 = 9003;

    const TIMER_TYPEWRITER: usize = 1;
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const ARP_KEY_PREFIX: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";

    // STATIC styles are not re-exported under UI in windows-sys 0.61.
    const SS_NOTIFY: u32 = 0x0100;
    const BST_CHECKED_I: isize = 1;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn pcw(v: &[u16]) -> PCWSTR {
        v.as_ptr()
    }
    fn colorref(rgb: (u8, u8, u8)) -> COLORREF {
        rgb.0 as u32 | ((rgb.1 as u32) << 8) | ((rgb.2 as u32) << 16)
    }
    fn set_text(hwnd: HWND, text: &str) {
        let w = wide(text);
        let _ = unsafe { SetWindowTextW(hwnd, pcw(&w)) };
    }
    fn get_text(hwnd: HWND) -> String {
        let mut buf = vec![0u16; 2048];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
    fn is_checked(hwnd: HWND) -> bool {
        if hwnd.is_null() {
            return false;
        }
        unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED_I }
    }
    fn set_checked(hwnd: HWND, on: bool) {
        if !hwnd.is_null() {
            let _ = unsafe { SendMessageW(hwnd, BM_SETCHECK, if on { 1 } else { 0 }, 0) };
        }
    }
    fn set_enabled(hwnd: HWND, on: bool) {
        if !hwnd.is_null() {
            let _ = unsafe { EnableWindow(hwnd, if on { 1 } else { 0 }) };
        }
    }

    // ------------------------------------------------------------------
    // Config loading
    // ------------------------------------------------------------------

    /// Demo fallback when this exe carries no payload.
    fn demo_config() -> ProjectConfig {
        let mut cfg = ProjectConfig::default();
        cfg.app.name = "RustPacker Demo".into();
        cfg.app.app_id = "RustPackerDemo".into();
        cfg.app.main_exe = "RustPackerDemo.exe".into();
        cfg.app.description =
            "Demo mode: this stub was run without a packed payload, so it installs a placeholder file."
                .into();
        cfg
    }

    /// Read the embedded project config from our own exe.
    ///
    /// Demo mode is entered **only** when this binary genuinely carries
    /// no payload. A packed installer whose payload is corrupt, whose
    /// config is missing/invalid, or whose config fails validation is a
    /// hard error: silently installing a placeholder app instead would
    /// hide tampering and leave a bogus Apps & Features entry behind.
    fn load_config() -> Result<(ProjectConfig, bool), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("could not locate this installer: {e}"))?;
        let bytes =
            std::fs::read(&exe).map_err(|e| format!("could not read this installer: {e}"))?;
        if find_payload(&bytes).is_err() {
            // Genuinely not packed: demo mode is intentional here.
            return Ok((demo_config(), false));
        }
        let manifest = read_manifest(&bytes).map_err(|e| {
            format!("this installer's payload is damaged and cannot be read:\n\n{e}")
        })?;
        let json = manifest.config_json.ok_or(
            "this installer's payload has no project configuration, so it cannot be installed.\n\
             Rebuild it with `rustpacker-cli pack`.",
        )?;
        let cfg = ProjectConfig::from_json(&json).map_err(|e| {
            format!("this installer's embedded project configuration is invalid:\n\n{e}")
        })?;
        let errors = cfg.validate();
        if !errors.is_empty() {
            return Err(format!(
                "this installer's embedded project configuration failed validation:\n\n  - {}",
                errors.join("\n  - ")
            ));
        }
        Ok((cfg, true))
    }

    fn sanitize_folder(name: &str) -> String {
        let cleaned: String = name
            .chars()
            .map(|c| {
                if "<>:\"/\\|?*".contains(c) || (c as u32) < 32 {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let trimmed = cleaned.trim().trim_end_matches('.').trim();
        if trimmed.is_empty() {
            "App".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Default install dir: per-user `%LOCALAPPDATA%\Programs\<App>`,
    /// per-machine `%ProgramFiles%\<App>`, honouring an absolute
    /// `install.default_dir` from the project file when present.
    fn default_install_dir(cfg: &ProjectConfig) -> String {
        let configured = cfg.install.default_dir.trim();
        if Path::new(configured).is_absolute() {
            return configured.to_string();
        }
        let per_machine = cfg.install.scope.resolve(is_elevated()).is_machine_wide()
            || configured == "{per-machine-programs}";
        let root = if per_machine {
            std::env::var_os("ProgramFiles").map(PathBuf::from)
        } else {
            std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Programs"))
        };
        let root = root.unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
        root.join(sanitize_folder(&cfg.app.name))
            .to_string_lossy()
            .into_owned()
    }

    // ------------------------------------------------------------------
    // Prerequisites, running-process checks, and PATH helpers
    // ------------------------------------------------------------------

    /// Expand `%VAR%` references in a probe path (case-insensitive).
    fn expand_env(raw: &str) -> String {
        let chars: Vec<char> = raw.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '%' {
                if let Some(close) = chars[i + 1..].iter().position(|&c| c == '%') {
                    let name: String = chars[i + 1..i + 1 + close].iter().collect();
                    if let Some(val) = std::env::var_os(&name) {
                        out.push_str(&val.to_string_lossy());
                        i += close + 2;
                        continue;
                    }
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    fn reg_key_exists(root: HKEY, subkey: &str) -> bool {
        let wsub = wide(subkey);
        let mut hk: HKEY = std::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(root, pcw(&wsub), 0, KEY_READ, &mut hk) };
        if rc == ERROR_SUCCESS {
            let _ = unsafe { RegCloseKey(hk) };
            true
        } else {
            false
        }
    }

    /// Evaluate one prerequisite. Returns `None` when it is satisfied and
    /// `Some(message)` when it is not.
    ///
    /// `registry_probe` accepts `HKLM\Path\To\Key` (key must exist) or
    /// `HKLM\Path\To\Key\ValueName` (value must exist).
    fn unmet_prerequisite(pr: &Prerequisite) -> Option<String> {
        let probe = pr.registry_probe.trim();
        if !probe.is_empty() {
            let mut parts = probe.splitn(2, '\\');
            let root_name = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("");
            let (key, value) = match rest.rsplit_once('\\') {
                Some((k, v)) if !k.is_empty() && !v.is_empty() => (k.to_string(), v.to_string()),
                _ => (rest.to_string(), String::new()),
            };
            let root = root_hkey(root_name);
            let found = if value.is_empty() {
                reg_key_exists(root, &key)
            } else {
                reg_get_sz(root, &key, &value).is_some()
            };
            if !found {
                return Some(pr.message.clone());
            }
        }
        let file_probe = pr.file_probe.trim();
        if !file_probe.is_empty() && !Path::new(&expand_env(file_probe)).exists() {
            return Some(pr.message.clone());
        }
        None
    }

    /// True when a process with this image name is running.
    fn process_running(image_name: &str) -> bool {
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot as isize == -1 {
            return false;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(image_name) {
                found = true;
                break;
            }
            ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        let _ = unsafe { windows_sys::Win32::Foundation::CloseHandle(snapshot) };
        found
    }

    /// Current Windows build number (e.g. 22631), or 0 when unknown.
    fn windows_build() -> u32 {
        use windows_sys::Win32::System::SystemInformation::{
            GetVersionExW, OSVERSIONINFOW,
        };
        let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        let rc = unsafe { GetVersionExW(&mut info) };
        if rc == 0 {
            return 0;
        }
        info.dwBuildNumber
    }

    /// Blocking pre-flight checks. Returns a user-facing reason to abort.
    fn preflight_blocker(cfg: &ProjectConfig) -> Option<String> {
        let running: Vec<String> = cfg
            .process_checks
            .iter()
            .filter(|pc| process_running(pc.process_name.trim()))
            .map(|pc| format!("{} — {}", pc.process_name.trim(), pc.message.trim()))
            .collect();
        if !running.is_empty() {
            return Some(format!(
                "Close the following before installing, then run Setup again:\n\n{}",
                running.join("\n\n")
            ));
        }
        if cfg.install.min_windows_build > 0 {
            let build = windows_build();
            if build > 0 && build < cfg.install.min_windows_build {
                return Some(format!(
                    "This application needs Windows build {} or newer; this PC reports build {build}.",
                    cfg.install.min_windows_build
                ));
            }
        }
        let unmet: Vec<String> = cfg
            .prerequisites
            .iter()
            .filter_map(unmet_prerequisite)
            .collect();
        if !unmet.is_empty() {
            return Some(format!(
                "This application needs software that is not installed:\n\n{}",
                unmet.join("\n\n")
            ));
        }
        None
    }

    // ------------------------------------------------------------------
    // Registry helpers (best-effort variants return Result for install)
    // ------------------------------------------------------------------

    fn root_hkey(root: &str) -> HKEY {
        if root.eq_ignore_ascii_case("HKLM") {
            HKEY_LOCAL_MACHINE
        } else {
            HKEY_CURRENT_USER
        }
    }

    fn reg_create(root: HKEY, subkey: &str) -> Result<HKEY, String> {
        let wsub = wide(subkey);
        let mut hk: HKEY = std::ptr::null_mut();
        let rc = unsafe {
            RegCreateKeyExW(
                root,
                pcw(&wsub),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut hk,
                std::ptr::null_mut(),
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("could not open registry key {subkey} (error {rc})"));
        }
        Ok(hk)
    }

    fn reg_open_write(root: HKEY, subkey: &str) -> Option<HKEY> {
        let wsub = wide(subkey);
        let mut hk: HKEY = std::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(root, pcw(&wsub), 0, KEY_SET_VALUE, &mut hk) };
        if rc == ERROR_SUCCESS {
            Some(hk)
        } else {
            None
        }
    }

    fn reg_set_sz(hk: HKEY, name: &str, value: &str) -> Result<(), String> {
        let wname = wide(name);
        let wval = wide(value);
        let rc = unsafe {
            RegSetValueExW(
                hk,
                pcw(&wname),
                0,
                REG_SZ,
                wval.as_ptr() as *const u8,
                (wval.len() * 2) as u32,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!(
                "could not write registry value {name} (error {rc})"
            ));
        }
        Ok(())
    }

    fn reg_set_sz_at(root: HKEY, subkey: &str, name: &str, value: &str) -> Result<(), String> {
        let hk = reg_create(root, subkey)?;
        let result = reg_set_sz(hk, name, value);
        let _ = unsafe { RegCloseKey(hk) };
        result
    }

    fn reg_delete_value(root: HKEY, subkey: &str, name: &str) {
        if let Some(hk) = reg_open_write(root, subkey) {
            let wname = wide(name);
            let _ = unsafe { RegDeleteValueW(hk, pcw(&wname)) };
            let _ = unsafe { RegCloseKey(hk) };
        }
    }

    fn reg_delete_tree(root: HKEY, subkey: &str) {
        let wsub = wide(subkey);
        let _ = unsafe { RegDeleteTreeW(root, pcw(&wsub)) };
    }

    /// Read an existing `REG_SZ` value, or `None` when it does not exist.
    fn reg_get_sz(root: HKEY, subkey: &str, name: &str) -> Option<String> {
        let wsub = wide(subkey);
        let mut hk: HKEY = std::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(root, pcw(&wsub), 0, KEY_READ, &mut hk) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let wname = wide(name);
        let mut ty: u32 = 0;
        let mut len: u32 = 0;
        let rc = unsafe {
            RegQueryValueExW(
                hk,
                pcw(&wname),
                std::ptr::null_mut(),
                &mut ty,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if rc != ERROR_SUCCESS || ty != REG_SZ || len == 0 || len > 64 * 1024 {
            let _ = unsafe { RegCloseKey(hk) };
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let rc = unsafe {
            RegQueryValueExW(
                hk,
                pcw(&wname),
                std::ptr::null_mut(),
                &mut ty,
                buf.as_mut_ptr(),
                &mut len,
            )
        };
        let _ = unsafe { RegCloseKey(hk) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let units: Vec<u16> = buf[..len as usize]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    /// Ask Windows to delete `path` on the next boot (used for the running
    /// uninstaller). Returns whether the request succeeded.
    fn schedule_delete_on_reboot(path: &Path) -> bool {
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
        let wpath = wide(&path.to_string_lossy());
        let rc = unsafe {
            MoveFileExW(
                pcw(&wpath),
                std::ptr::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )
        };
        rc != 0
    }

    /// Remove the Apps & Features key for an app (used when an install is
    /// abandoned before its journal exists).
    fn remove_arp_entry(arp_root_name: &str, arp_key: &str) {
        reg_delete_tree(root_hkey(arp_root_name), arp_key);
    }

    /// Undo a journal in reverse order. Returns a list of items that
    /// could not be undone. Paths outside `install_dir` are refused, so
    /// a tampered journal can never delete unrelated files.
    ///
    /// Directories are removed after every file (they are only removed
    /// when empty), which fixes the old reverse-order bug where a
    /// parent folder was tried before the files inside it.
    fn rollback_journal(journal: &InstallJournal, install_dir: &str) -> Vec<String> {
        let mut failures: Vec<String> = Vec::new();
        let base = Path::new(install_dir);
        let confined = |p: &str| journal.is_path_confined(p);
        let dirs: Vec<String> = journal
            .rollback_plan()
            .into_iter()
            .filter_map(|a| match a {
                JournalAction::CreateDir { path } => Some(path.clone()),
                _ => None,
            })
            .collect();
        for action in journal.rollback_plan() {
            match action {
                JournalAction::CreateFile { path } | JournalAction::CreateShortcut { path } => {
                    if !confined(path) {
                        failures.push(format!("refused to delete outside the install folder: {path}"));
                        continue;
                    }
                    if Path::new(path).is_file() {
                        if let Err(e) = std::fs::remove_file(path) {
                            failures.push(format!("{path}: {e}"));
                        }
                    }
                }
                JournalAction::OverwriteFile { path, backup_name } => {
                    if !confined(path) {
                        failures.push(format!("refused to restore outside the install folder: {path}"));
                        continue;
                    }
                    let backup = base.join(backup_name);
                    if backup.is_file() {
                        let _ = std::fs::remove_file(path);
                        if let Err(e) = std::fs::rename(&backup, path) {
                            failures.push(format!("{path}: {e}"));
                        }
                    }
                }
                JournalAction::RegistryValue {
                    root,
                    key,
                    name,
                    previous_value,
                } => {
                    let hk_root = root_hkey(root);
                    match previous_value {
                        Some(prev) => {
                            let _ = reg_set_sz_at(hk_root, key, name, prev);
                        }
                        None => reg_delete_value(hk_root, key, name),
                    }
                }
                JournalAction::RunEntry { name } => {
                    reg_delete_value(HKEY_CURRENT_USER, RUN_KEY, name);
                }
                JournalAction::ArpEntry { root, key } => {
                    reg_delete_tree(root_hkey(root), key);
                }
                JournalAction::FileAssociation {
                    extension,
                    prog_id,
                    previous_existed,
                } => {
                    if !previous_existed {
                        let classes = "Software\\Classes";
                        reg_delete_tree(HKEY_CURRENT_USER, &format!("{classes}\\{extension}"));
                        reg_delete_tree(HKEY_CURRENT_USER, &format!("{classes}\\{prog_id}"));
                    }
                }
                JournalAction::PathEntry { scope, added_dir } => {
                    // PATH is rewritten by the RegistryValue entry recorded
                    // alongside this one (which restores the old value or
                    // deletes the value when it did not exist). This arm
                    // only exists to remove the folder from a PATH that a
                    // *foreign* journal appended it to.
                    let root = root_hkey(scope);
                    if let Some(current) = reg_get_sz(root, "Environment", "Path") {
                        let kept: Vec<&str> = current
                            .split(';')
                            .filter(|p| !p.trim().eq_ignore_ascii_case(added_dir.trim()))
                            .collect();
                        if kept.len() != current.split(';').count() {
                            let _ = reg_set_sz_at(root, "Environment", "Path", &kept.join(";"));
                        }
                    }
                }
                JournalAction::CreateDir { .. } => {}
            }
        }
        // Any journal this installer wrote into the folder is ours to
        // remove (the caller re-saves it on success).
        let journal_in_dir = base.join(JOURNAL_FILE_NAME);
        if journal_in_dir.is_file() {
            let _ = std::fs::remove_file(&journal_in_dir);
        }
        // Backups that were not consumed by a restore (or that belonged to
        // files already gone) are installer artefacts and must not keep a
        // folder from being removed.
        let backup_prefix = format!("{JOURNAL_FILE_NAME}.");
        if let Ok(read) = std::fs::read_dir(base) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&backup_prefix) && name.ends_with(".bak") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        // Files are gone now: remove directories deepest-first so a
        // parent is only attempted after its children.
        let mut sorted = dirs;
        sorted.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for dir in sorted {
            if !confined(&dir) {
                continue;
            }
            // `remove_dir` only succeeds on an empty folder, so a user's
            // pre-existing folder is never removed here.
            let _ = std::fs::remove_dir(&dir);
        }
        failures
    }

    // ------------------------------------------------------------------
    // Desktop shortcut via IShellLinkW / IPersistFile COM.
    //
    // windows-sys ships the CLSID and the COM functions but not the
    // interface vtables, so the two vtables are declared here with the
    // standard documented layout. Only the slots we call are typed;
    // the rest are opaque pointer-sized placeholders.
    // ------------------------------------------------------------------

    const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

    type HResultFn = unsafe extern "system" fn(*mut c_void) -> HRESULT;
    #[repr(C)]
    struct IUnknownVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }
    #[repr(C)]
    struct IShellLinkWVtbl {
        base: IUnknownVtbl,
        get_path: *const c_void,
        get_id_list: *const c_void,
        set_id_list: *const c_void,
        get_description: *const c_void,
        set_description: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
        get_working_directory: *const c_void,
        set_working_directory: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
        get_arguments: *const c_void,
        set_arguments: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
        get_hotkey: *const c_void,
        set_hotkey: *const c_void,
        get_show_cmd: *const c_void,
        set_show_cmd: *const c_void,
        get_icon_location: *const c_void,
        set_icon_location: *const c_void,
        set_relative_path: *const c_void,
        resolve: *const c_void,
        set_path: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    }
    #[repr(C)]
    struct IPersistFileVtbl {
        base: IUnknownVtbl,
        get_class_id: *const c_void,
        is_dirty: HResultFn,
        load: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
        save: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
        save_completed: *const c_void,
        get_cur_file: *const c_void,
    }
    fn com_vtbl<T>(obj: *mut c_void) -> *const T {
        unsafe { *(obj as *const *const T) }
    }
    unsafe fn com_release(obj: *mut c_void) {
        if !obj.is_null() {
            let vtbl = com_vtbl::<IUnknownVtbl>(obj);
            let _ = ((*vtbl).release)(obj);
        }
    }
    unsafe fn com_query(obj: *mut c_void, iid: &GUID) -> Result<*mut c_void, String> {
        let vtbl = com_vtbl::<IUnknownVtbl>(obj);
        let mut out: *mut c_void = std::ptr::null_mut();
        let hr = ((*vtbl).query_interface)(obj, iid, &mut out);
        if hr < 0 || out.is_null() {
            return Err(format!("shortcut COM query failed (0x{:08x})", hr as u32));
        }
        Ok(out)
    }

    /// Resolve a shell known folder (Desktop, Start Menu, ...).
    fn known_folder(csidl: i32, label: &str) -> Result<PathBuf, String> {
        let mut buf = vec![0u16; 1024];
        let hr = unsafe {
            SHGetFolderPathW(
                std::ptr::null_mut(),
                csidl,
                std::ptr::null_mut(),
                0,
                buf.as_mut_ptr(),
            )
        };
        if hr != 0 {
            return Err(format!(
                "could not find the {label} folder (0x{:08x})",
                hr as u32
            ));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len == 0 {
            return Err(format!("the {label} folder path is empty"));
        }
        Ok(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }

    fn desktop_dir() -> Result<PathBuf, String> {
        known_folder(CSIDL_DESKTOPDIRECTORY as i32, "Desktop")
    }

    fn start_menu_dir() -> Result<PathBuf, String> {
        known_folder(CSIDL_PROGRAMS as i32, "Start Menu")
    }

    /// Create a `.lnk` pointing at `target` on the Desktop. Returns the link path.
    fn create_shortcut(
        target: &Path,
        link_name: &str,
        working_dir: &Path,
        description: &str,
    ) -> Result<PathBuf, String> {
        let folder = desktop_dir()?;
        create_shortcut_in(&folder, target, link_name, working_dir, description, "")
    }

    /// Create a `.lnk` for a project `[[shortcuts]]` entry, honouring
    /// `desktop` / `start-menu` (validated by the caller).
    fn create_shortcut_at(
        location: &str,
        target: &Path,
        link_name: &str,
        working_dir: &Path,
        arguments: &str,
    ) -> Result<PathBuf, String> {
        let folder = if location.trim().eq_ignore_ascii_case("desktop") {
            desktop_dir()?
        } else {
            start_menu_dir()?
        };
        create_shortcut_in(&folder, target, link_name, working_dir, link_name, arguments)
    }

    fn create_shortcut_in(
        folder: &Path,
        target: &Path,
        link_name: &str,
        working_dir: &Path,
        description: &str,
        arguments: &str,
    ) -> Result<PathBuf, String> {
        let safe_name = sanitize_folder(link_name);
        let link_path = folder.join(format!("{safe_name}.lnk"));
        let hr_init = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        // S_OK (0) or S_FALSE (1): we own a CoUninitialize. RPC_E_CHANGED_MODE
        // means COM was already initialised differently; proceed without it.
        let we_initialised = hr_init >= 0;
        let result =
            create_shortcut_inner(target, &link_path, working_dir, description, arguments);
        if we_initialised {
            unsafe { CoUninitialize() };
        }
        result.map(|()| link_path)
    }

    fn create_shortcut_inner(
        target: &Path,
        link_path: &Path,
        working_dir: &Path,
        description: &str,
        arguments: &str,
    ) -> Result<(), String> {
        unsafe {
            let clsid = windows_sys::Win32::UI::Shell::ShellLink;
            let mut obj: *mut c_void = std::ptr::null_mut();
            let hr = CoCreateInstance(
                &clsid,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ISHELL_LINK_W,
                &mut obj,
            );
            if hr < 0 || obj.is_null() {
                return Err(format!(
                    "could not create the shortcut object (0x{:08x})",
                    hr as u32
                ));
            }
            let outcome = (|| -> Result<(), String> {
                let vtbl = com_vtbl::<IShellLinkWVtbl>(obj);
                let wtarget = wide(&target.to_string_lossy());
                let hr = ((*vtbl).set_path)(obj, pcw(&wtarget));
                if hr < 0 {
                    return Err(format!("shortcut SetPath failed (0x{:08x})", hr as u32));
                }
                let wdir = wide(&working_dir.to_string_lossy());
                let _ = ((*vtbl).set_working_directory)(obj, pcw(&wdir));
                let wdesc = wide(description);
                let _ = ((*vtbl).set_description)(obj, pcw(&wdesc));
                let wargs = wide(arguments);
                let _ = ((*vtbl).set_arguments)(obj, pcw(&wargs));
                let persist = com_query(obj, &IID_IPERSIST_FILE)?;
                let pvtbl = com_vtbl::<IPersistFileVtbl>(persist);
                let wlink = wide(&link_path.to_string_lossy());
                let hr = ((*pvtbl).save)(persist, pcw(&wlink), 1);
                com_release(persist);
                if hr < 0 {
                    return Err(format!("shortcut save failed (0x{:08x})", hr as u32));
                }
                Ok(())
            })();
            com_release(obj);
            outcome
        }
    }

    // ------------------------------------------------------------------
    // Install
    // ------------------------------------------------------------------

    struct InstallOutcome {
        journal: InstallJournal,
        install_dir: String,
    }

    /// Extract the payload, copy the uninstaller, write the ARP entry,
    /// and save the journal. `report(percent, status)` updates the page.
    fn perform_install(
        cfg: &ProjectConfig,
        install_dir: &str,
        mut report: impl FnMut(u32, &str),
    ) -> Result<InstallOutcome, String> {
        // Any failure after the first change rolls everything back, so a
        // half-installed app never reaches Apps & Features. The plan is
        // journal-driven, so it undoes exactly what was done (including
        // restored overwritten files and registry values).
        let effective_scope = cfg.install.scope.resolve(is_elevated());
        let mut journal = InstallJournal::new(
            &cfg.app.app_id,
            &cfg.app.name,
            &cfg.app.version,
            install_dir,
            effective_scope.label(),
        );
        match perform_install_inner(cfg, install_dir, effective_scope, &mut journal, &mut report) {
            Ok(()) => Ok(InstallOutcome {
                journal,
                install_dir: install_dir.to_string(),
            }),
            Err(e) => {
                // A per-machine install that failed *because we are not
                // elevated* can often succeed after a UAC re-launch.
                let wants_machine = cfg.install.scope.resolve(is_elevated()).is_machine_wide()
                    || matches!(cfg.install.scope, InstallScope::PerMachine);
                if !is_elevated() && wants_machine {
                    let undo_failures = rollback_journal(&journal, install_dir);
                    let detail = if undo_failures.is_empty() { String::new() } else {
                        format!("\nSome items may remain:\n  - {}", undo_failures.join("\n  - "))
                    };
                    return Err(format!(
                        "{e}\n\nThis project installs for all users (per-machine), which needs \
                         administrator rights. Windows asked for elevation, but it was not granted.\
                         {detail}"
                    ));
                }
                report(0, "Rolling back...");
                let undo_failures = rollback_journal(&journal, install_dir);
                if undo_failures.is_empty() {
                    Err(format!("{e}\n\nEvery change made by the installer was rolled back."))
                } else {
                    Err(format!(
                        "{e}\n\nRollback was incomplete; these items may remain:\n  - {}",
                        undo_failures.join("\n  - ")
                    ))
                }
            }
        }
    }

    fn perform_install_inner(
        cfg: &ProjectConfig,
        install_dir: &str,
        effective_scope: InstallScope,
        journal: &mut InstallJournal,
        report: &mut impl FnMut(u32, &str),
    ) -> Result<(), String> {
        let dest = PathBuf::from(install_dir);
        let own_exe =
            std::env::current_exe().map_err(|e| format!("could not locate this installer: {e}"))?;
        let per_machine = effective_scope.is_machine_wide();
        let (arp_root, arp_root_name) = if per_machine {
            (HKEY_LOCAL_MACHINE, "HKLM")
        } else {
            (HKEY_CURRENT_USER, "HKCU")
        };

        report(5, "Creating the install folder...");
        // `create_new` tells us whether the folder already existed, so
        // rollback never deletes a pre-existing user folder.
        let folder_preexisted = dest.exists();
        std::fs::create_dir_all(&dest)
            .map_err(|e| format!("could not create {}: {e}", dest.display()))?;
        if !folder_preexisted {
            journal.record_dir(install_dir);
        }

        report(20, "Extracting files...");
        let exe_bytes =
            std::fs::read(&own_exe).map_err(|e| format!("could not read this installer: {e}"))?;
        let mut recorded_dirs: HashSet<String> = HashSet::new();
        recorded_dirs.insert(install_dir.to_string());
        if find_payload(&exe_bytes).is_ok() {
            // Read the manifest first so folders and overwrite backups are
            // journaled *before* `extract_payload` writes anything: a
            // failure part-way through then rolls back every folder the
            // extractor created, including ones already written.
            let manifest = read_manifest(&exe_bytes)
                .map_err(|e| format!("could not read the payload: {e}"))?;
            for entry in &manifest.files {
                let full = dest.join(entry.path.replace('/', "\\"));
                if full.is_file() {
                    let backup_name = format!(
                        "{}.{}.bak",
                        JOURNAL_FILE_NAME,
                        sha256_hex(entry.path.as_bytes())
                    );
                    let backup_path = dest.join(&backup_name);
                    if !backup_path.exists() {
                        std::fs::copy(&full, &backup_path).map_err(|e| {
                            format!("could not back up existing {}: {e}", full.display())
                        })?;
                        journal.record(JournalAction::OverwriteFile {
                            path: full.to_string_lossy().into_owned(),
                            backup_name,
                        });
                    }
                }
                let mut parents: Vec<PathBuf> = Vec::new();
                let mut parent = full.parent().map(|p| p.to_path_buf());
                while let Some(dir) = parent {
                    if dir == dest || dir.parent().is_none() {
                        break;
                    }
                    parents.push(dir.clone());
                    parent = dir.parent().map(|p| p.to_path_buf());
                }
                for dir in parents.into_iter().rev() {
                    if !dir.exists() {
                        let key = dir.to_string_lossy().into_owned();
                        if recorded_dirs.insert(key.clone()) {
                            journal.record_dir(&key);
                        }
                    }
                }
                journal.record_file(&full.to_string_lossy());
            }
            let extracted = extract_payload(&exe_bytes, &dest)?;
            debug_assert_eq!(extracted.len(), manifest.files.len());
            // The app the installer promises must actually be there.
            let main_exe = dest.join(&cfg.app.main_exe);
            if !main_exe.is_file() {
                return Err(format!(
                    "the payload does not contain app.main_exe '{}'; the project file and \
                     install.source_dir do not match",
                    cfg.app.main_exe
                ));
            }
        } else {
            // Demo mode: leave a placeholder so the install is real.
            let note = dest.join("README-RustPackerDemo.txt");
            std::fs::write(
                &note,
                format!(
                    "{} {}\r\nInstalled by the RustPacker stub in demo mode (no payload was packed).\r\n",
                    cfg.app.name, cfg.app.version
                ),
            )
            .map_err(|e| format!("could not write {}: {e}", note.display()))?;
            journal.record_file(&note.to_string_lossy());
        }

        report(65, "Installing the uninstaller...");
        let uninstaller = dest.join(UNINSTALLER_FILE_NAME);
        std::fs::copy(&own_exe, &uninstaller)
            .map_err(|e| format!("could not install the uninstaller: {e}"))?;
        journal.record_file(&uninstaller.to_string_lossy());

        report(80, "Registering in Apps & Features...");
        let arp_key = format!("{ARP_KEY_PREFIX}\\{}", cfg.app.app_id);
        let hk = reg_create(arp_root, &arp_key)?;
        let reg_result = (|| -> Result<(), String> {
            reg_set_sz(hk, "DisplayName", &cfg.app.name)?;
            reg_set_sz(hk, "DisplayVersion", &cfg.app.version)?;
            reg_set_sz(hk, "Publisher", &cfg.app.publisher)?;
            reg_set_sz(hk, "InstallLocation", install_dir)?;
            reg_set_sz(
                hk,
                "UninstallString",
                &format!("\"{}\"", uninstaller.to_string_lossy()),
            )?;
            reg_set_sz(hk, "DisplayIcon", &uninstaller.to_string_lossy())?;
            Ok(())
        })();
        let _ = unsafe { RegCloseKey(hk) };
        reg_result?;
        journal.record(JournalAction::ArpEntry {
            root: arp_root_name.to_string(),
            key: arp_key,
        });

        report(86, "Applying project registry entries...");
        for entry in &cfg.registry {
            let hk_root = root_hkey(&entry.root);
            // Record the previous value first so uninstall can restore it.
            let previous = reg_get_sz(hk_root, &entry.key, &entry.name);
            reg_set_sz_at(hk_root, &entry.key, &entry.name, &entry.value)
                .map_err(|e| format!("could not write registry value {}: {e}", entry.name))?;
            journal.record(JournalAction::RegistryValue {
                root: entry.root.clone(),
                key: entry.key.clone(),
                name: entry.name.clone(),
                previous_value: previous,
            });
        }

        report(89, "Registering file associations...");
        for assoc in &cfg.file_associations {
            let open_exe = dest.join(assoc.open_with.trim());
            if !open_exe.is_file() {
                return Err(format!(
                    "file association {} points at '{}', which the payload does not install",
                    assoc.extension, assoc.open_with
                ));
            }
            let classes = "Software\\Classes";
            let ext_key = format!("{classes}\\{}", assoc.extension);
            let prog_key = format!("{classes}\\{}", assoc.prog_id);
            let previous_existed = reg_key_exists(HKEY_CURRENT_USER, &ext_key)
                || reg_key_exists(HKEY_CURRENT_USER, &prog_key);
            reg_set_sz_at(HKEY_CURRENT_USER, &ext_key, "", assoc.prog_id.trim())
                .map_err(|e| format!("could not associate {}: {e}", assoc.extension))?;
            reg_set_sz_at(HKEY_CURRENT_USER, &prog_key, "", assoc.description.trim())
                .map_err(|e| format!("could not register {}: {e}", assoc.prog_id))?;
            reg_set_sz_at(
                HKEY_CURRENT_USER,
                &format!("{prog_key}\\shell\\open\\command"),
                "",
                &format!("\"{}\" \"%1\"", open_exe.to_string_lossy()),
            )
            .map_err(|e| format!("could not register the open command: {e}"))?;
            journal.record(JournalAction::FileAssociation {
                extension: assoc.extension.clone(),
                prog_id: assoc.prog_id.clone(),
                previous_existed,
            });
        }

        if !cfg.path_scope.trim().is_empty() {
            report(90, "Adding the install folder to PATH...");
            // A per-user install must never write the machine PATH (it
            // cannot, and a mismatch is a project-validation error).
            let wants_machine = cfg.path_scope.trim().eq_ignore_ascii_case("machine");
            let (root, root_name) = if wants_machine && per_machine {
                (HKEY_LOCAL_MACHINE, "HKLM")
            } else {
                (HKEY_CURRENT_USER, "HKCU")
            };
            let path_key = "Environment";
            let previous = reg_get_sz(root, path_key, "Path");
            let already = previous
                .as_deref()
                .map(|p| {
                    p.split(';')
                        .any(|part| part.trim().trim_end_matches('\\').eq_ignore_ascii_case(
                            install_dir.trim().trim_end_matches('\\'),
                        ))
                })
                .unwrap_or(false);
            if !already {
                let base = previous.clone().unwrap_or_default();
                let new_path = if base.trim().is_empty() {
                    install_dir.to_string()
                } else {
                    format!("{};{install_dir}", base.trim_end_matches(';'))
                };
                reg_set_sz_at(root, path_key, "Path", &new_path)
                    .map_err(|e| format!("could not add the install folder to PATH: {e}"))?;
                journal.record(JournalAction::RegistryValue {
                    root: root_name.to_string(),
                    key: path_key.to_string(),
                    name: "Path".to_string(),
                    previous_value: previous,
                });
                journal.record(JournalAction::PathEntry {
                    scope: root_name.to_string(),
                    added_dir: install_dir.to_string(),
                });
            }
        }

        report(92, "Saving the install journal...");
        journal.record_file(&dest.join(JOURNAL_FILE_NAME).to_string_lossy());
        let journal_path = dest.join(JOURNAL_FILE_NAME);
        if let Err(e) = journal.save(&journal_path) {
            // Without a journal there is no reliable uninstaller, so the
            // install is undone rather than left in a half-managed state.
            remove_arp_entry(arp_root_name, &arp_key);
            return Err(format!(
                "could not save the install journal ({e}), so the installation was cancelled"
            ));
        }

        report(100, "Installation complete.");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Uninstall (journal rollback)
    // ------------------------------------------------------------------

    /// True when this process can write to `HKLM` (i.e. it holds an
    /// elevated token). Probing the registry this way avoids a second
    /// API surface and answers the question the installer actually asks.
    fn is_elevated() -> bool {
        let sub = wide("SOFTWARE");
        let mut hk: HKEY = std::ptr::null_mut();
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                pcw(&sub),
                0,
                KEY_WRITE,
                &mut hk,
            )
        };
        if rc != ERROR_SUCCESS {
            return false;
        }
        let _ = unsafe { RegCloseKey(hk) };
        true
    }

    /// Re-launch this installer elevated (UAC) so a per-machine install
    /// can proceed. Returns true when the elevated copy was started.
    fn relaunch_elevated() -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let wfile = wide(&exe.to_string_lossy());
        let verb = windows_sys::core::w!("runas");
        let rc = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb,
                pcw(&wfile),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW returns a value > 32 on success.
        rc as isize > 32
    }

    fn message_box(text: &str, caption: &str, error: bool) {
        let wtext = wide(text);
        let wcap = wide(caption);
        let style = MB_OK
            | if error {
                MB_ICONERROR
            } else {
                MB_ICONINFORMATION
            };
        let _ = unsafe { MessageBoxW(std::ptr::null_mut(), pcw(&wtext), pcw(&wcap), style) };
    }

    /// Undo everything in the journal beside this exe, in rollback order:
    /// files and shortcuts first, then Run/registry entries and the ARP
    /// key, then directories. Returns a process exit code.
    ///
    /// Every path is confined to the journal's install directory, so a
    /// tampered or copied journal cannot delete files elsewhere. When an
    /// interactive uninstall finds a journal that tries to escape, it
    /// refuses to proceed instead of asking the user to trust it.
    fn run_uninstall(silent: bool) -> i32 {
        let Some(own_exe) = std::env::current_exe().ok() else {
            if !silent {
                message_box("Could not locate this uninstaller.", "Uninstall", true);
            }
            return 1;
        };
        let Some(dir) = own_exe.parent().map(|p| p.to_path_buf()) else {
            if !silent {
                message_box("Could not locate the install folder.", "Uninstall", true);
            }
            return 1;
        };
        let journal_path = dir.join(JOURNAL_FILE_NAME);
        let journal = match InstallJournal::load(&journal_path) {
            Ok(j) => j,
            Err(e) => {
                if !silent {
                    message_box(
                        &format!("No install journal was found, so nothing was removed.\n\n{e}"),
                        "Uninstall",
                        true,
                    );
                }
                return 1;
            }
        };

        // Refuse a journal whose own location is not where this exe lives,
        // or whose entries point outside its install directory.
        let journal_dir_ok = journal.is_path_confined(&dir.to_string_lossy());
        let escapes = journal
            .entries
            .iter()
            .any(|e| match &e.action {
                JournalAction::CreateFile { path }
                | JournalAction::CreateShortcut { path }
                | JournalAction::OverwriteFile { path, .. }
                | JournalAction::CreateDir { path } => !journal.is_path_confined(path),
                _ => false,
            });
        if !journal_dir_ok || escapes {
            if !silent {
                message_box(
                    "The install journal is inconsistent with this folder, so the uninstall \
                     was stopped to avoid deleting the wrong files.\n\n\
                     Remove the application folder manually if you trust it.",
                    "Uninstall",
                    true,
                );
            }
            return 1;
        }

        if !silent {
            let wtext = wide(&format!(
                "Remove {} {} from this PC?",
                journal.app_name, journal.version
            ));
            let wcap = wide("Uninstall");
            let answer = unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    pcw(&wtext),
                    pcw(&wcap),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                )
            };
            if answer != IDYES {
                return 0;
            }
        }

        // The running uninstaller cannot delete itself while it runs; the
        // files are removed now and Windows deletes the exe at the next
        // boot (MoveFileExW with MOVEFILE_DELAY_UNTIL_REBOOT).
        let mut failures = rollback_journal(&journal, &journal.install_dir);
        let _ = std::fs::remove_file(&journal_path);
        let _ = std::fs::remove_dir(&dir);
        let _ = std::fs::remove_dir(&journal.install_dir);
        let queued = schedule_delete_on_reboot(&own_exe);
        if !queued {
            // Point the user at the one leftover file rather than
            // pretending the folder is gone.
            failures.push(format!(
                "{} could not be scheduled for deletion; delete it manually",
                own_exe.display()
            ));
        }

        if failures.is_empty() {
            if !silent {
                message_box(
                    &format!("{} was removed from this PC.", journal.app_name),
                    "Uninstall",
                    false,
                );
            }
            0
        } else {
            if !silent {
                message_box(
                    &format!(
                        "Uninstall finished, but some items could not be removed:\n\n{}",
                        failures.join("\n")
                    ),
                    "Uninstall",
                    true,
                );
            }
            1
        }
    }

    // ------------------------------------------------------------------
    // Wizard
    // ------------------------------------------------------------------

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Page {
        Welcome,
        License,
        Directory,
        Installing,
        Finish,
    }

    struct WizardState {
        cfg: ProjectConfig,
        packed: bool,
        pages: Vec<Page>,
        page_index: usize,
        typewriter: Typewriter,
        body_full: String,
        // Window + persistent controls.
        hwnd: HWND,
        title_hwnd: HWND,
        body_hwnd: HWND,
        back_hwnd: HWND,
        next_hwnd: HWND,
        cancel_hwnd: HWND,
        // Page controls (null when not on that page).
        dir_edit: HWND,
        progress: HWND,
        status: HWND,
        chk_desktop: HWND,
        chk_launch: HWND,
        chk_launch_tray: HWND,
        chk_login: HWND,
        chk_login_tray: HWND,
        // GDI resources.
        font: HFONT,
        title_font: HFONT,
        bg_brush: HBRUSH,
        surface_brush: HBRUSH,
        text_color: COLORREF,
        bg_color: COLORREF,
        // Install results.
        install_dir: String,
        journal: Option<InstallJournal>,
        install_done: bool,
        install_error: Option<String>,
    }

    thread_local! {
        static STATE: RefCell<Option<WizardState>> = const { RefCell::new(None) };
    }

    /// Run `f` against the wizard state if it is not already borrowed
    /// (window messages can arrive re-entrantly while controls are being
    /// created; those nested calls simply fall through to DefWindowProc).
    fn with_state<R>(f: impl FnOnce(&mut WizardState) -> R) -> Option<R> {
        STATE.with(|cell| {
            let mut slot = cell.try_borrow_mut().ok()?;
            let state = slot.as_mut()?;
            Some(f(state))
        })
    }
    fn with_state_ref<R>(f: impl FnOnce(&WizardState) -> R) -> Option<R> {
        STATE.with(|cell| {
            let slot = cell.try_borrow().ok()?;
            let state = slot.as_ref()?;
            Some(f(state))
        })
    }

    fn page_title(page: Page) -> &'static str {
        match page {
            Page::Welcome => "Welcome",
            Page::License => "License Agreement",
            Page::Directory => "Install Location",
            Page::Installing => "Installing",
            Page::Finish => "Finished",
        }
    }

    fn page_body(state: &WizardState, page: Page) -> String {
        let cfg = &state.cfg;
        match page {
            Page::Welcome => {
                let mut text = format!(
                    "Welcome to the {} {} setup wizard.\r\n\r\nPublisher: {}",
                    cfg.app.name, cfg.app.version, cfg.app.publisher
                );
                if !cfg.app.description.trim().is_empty() {
                    text.push_str("\r\n\r\n");
                    text.push_str(cfg.app.description.trim());
                }
                if !state.packed {
                    text.push_str(
                        "\r\n\r\n[Demo mode: no payload is packed in this stub, so a placeholder file will be installed.]",
                    );
                }
                text.push_str("\r\n\r\nPress Next to continue.");
                text
            }
            Page::License => {
                // `license_text` is embedded by the packer; a project that
                // only names a file (hand-edited config) falls back to a
                // clear notice instead of showing the user a bare path.
                let text = cfg.install.license_text.trim();
                if text.is_empty() {
                    format!(
                        "{} includes a license agreement ({}), but this installer was built \
                         without its text.\r\n\r\nPress Next if you accept the terms shipped \
                         with {}.",
                        cfg.app.name, cfg.install.license_file, cfg.app.name
                    )
                } else {
                    format!(
                        "License agreement for {} {}\r\n\r\n{}\r\n\r\n\
                         Press Next if you accept the agreement.",
                        cfg.app.name,
                        cfg.app.version,
                        text
                    )
                }
            }
            Page::Directory => format!(
                "Choose where to install {}.\r\n\r\n\
                 Setup will install the program into the folder below. \
                 You can type a different folder, then press Next.",
                cfg.app.name
            ),
            Page::Installing => {
                if let Some(err) = &state.install_error {
                    format!("Installation failed.\r\n\r\n{err}")
                } else if state.install_done {
                    format!(
                        "{} {} was installed successfully.\r\n\r\nPress Next to finish.",
                        cfg.app.name, cfg.app.version
                    )
                } else {
                    format!(
                        "Installing {} {}...\r\n\r\nPlease wait.",
                        cfg.app.name, cfg.app.version
                    )
                }
            }
            Page::Finish => format!(
                "Setup has finished installing {} on this PC.\r\n\r\n\
                 Choose any final actions below, then press Finish.",
                cfg.app.name
            ),
        }
    }

    const WS_CHILD_: u32 = 0x4000_0000;
    const WS_VISIBLE_: u32 = 0x1000_0000;
    const WS_TABSTOP_: u32 = 0x0001_0000;
    const WS_BORDER_: u32 = 0x0080_0000;

    unsafe fn make_child(
        parent: HWND,
        class: &str,
        text: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: i32,
        font: HFONT,
    ) -> HWND {
        let wclass = wide(class);
        let wtext = wide(text);
        let hwnd = CreateWindowExW(
            0,
            pcw(&wclass),
            pcw(&wtext),
            style,
            x,
            y,
            w,
            h,
            parent,
            id as usize as HMENU,
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        if !hwnd.is_null() && !font.is_null() {
            let _ = SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
        }
        hwnd
    }

    fn destroy_page_controls(state: &mut WizardState) {
        for slot in [
            &mut state.dir_edit,
            &mut state.progress,
            &mut state.status,
            &mut state.chk_desktop,
            &mut state.chk_launch,
            &mut state.chk_launch_tray,
            &mut state.chk_login,
            &mut state.chk_login_tray,
        ] {
            if !slot.is_null() {
                let _ = unsafe { DestroyWindow(*slot) };
                *slot = std::ptr::null_mut();
            }
        }
    }

    /// Show page `index`: rebuild page controls, restart the typewriter,
    /// refresh nav buttons, and (for Installing) run the install.
    fn show_page(index: usize) {
        // Snapshot everything the Win32 calls need, then drop the borrow:
        // creating controls sends messages back to our window procedure.
        let Some(snapshot) = with_state(|s| {
            s.page_index = index.min(s.pages.len().saturating_sub(1));
            let page = s.pages[s.page_index];
            s.body_full = page_body(s, page);
            s.typewriter = Typewriter::new(&s.body_full, s.cfg.typewriter_interval_ms);
            (
                s.hwnd,
                s.title_hwnd,
                s.body_hwnd,
                s.back_hwnd,
                s.next_hwnd,
                s.cancel_hwnd,
                s.font,
                page,
                s.pages.len(),
                s.page_index,
                s.cfg.clone(),
                s.install_dir.clone(),
                s.install_done,
                s.install_error.is_some(),
            )
        }) else {
            return;
        };
        let (
            hwnd,
            title,
            body,
            back,
            next,
            _cancel,
            font,
            page,
            page_count,
            page_index,
            cfg,
            install_dir,
            install_done,
            install_failed,
        ) = snapshot;

        with_state(|s| destroy_page_controls(s));
        set_text(title, page_title(page));
        set_text(body, "");
        let _ = unsafe { KillTimer(hwnd, TIMER_TYPEWRITER) };
        let interval = cfg.typewriter_interval_ms.max(1);
        let _ = unsafe { SetTimer(hwnd, TIMER_TYPEWRITER, interval, None) };

        // Nav buttons.
        set_enabled(back, page_index > 0 && page != Page::Installing);
        match page {
            Page::Installing => {
                set_text(next, "Next >");
                set_enabled(next, install_done);
                if install_failed {
                    set_enabled(back, true);
                }
            }
            Page::Finish => set_text(next, "Finish"),
            _ => set_text(next, "Next >"),
        }
        if page != Page::Installing && page != Page::Finish {
            set_enabled(next, true);
        }
        let _ = page_count;

        match page {
            Page::Directory => {
                let edit = unsafe {
                    make_child(
                        hwnd,
                        "EDIT",
                        &install_dir,
                        WS_CHILD_ | WS_VISIBLE_ | WS_TABSTOP_ | WS_BORDER_ | 0x0080, // ES_AUTOHSCROLL
                        24,
                        196,
                        512,
                        26,
                        ID_DIR_EDIT,
                        font,
                    )
                };
                with_state(|s| s.dir_edit = edit);
            }
            Page::Installing => {
                let progress = unsafe {
                    make_child(
                        hwnd,
                        "msctls_progress32",
                        "",
                        WS_CHILD_ | WS_VISIBLE_,
                        24,
                        200,
                        512,
                        22,
                        ID_PROGRESS,
                        font,
                    )
                };
                let status = unsafe {
                    make_child(
                        hwnd,
                        "STATIC",
                        "Preparing...",
                        WS_CHILD_ | WS_VISIBLE_,
                        24,
                        234,
                        512,
                        60,
                        ID_STATUS,
                        font,
                    )
                };
                with_state(|s| {
                    s.progress = progress;
                    s.status = status;
                });
                if !install_done && !install_failed {
                    run_install_on_page(cfg, install_dir, progress, status);
                }
            }
            Page::Finish => build_finish_controls(hwnd, font, &cfg),
            _ => {}
        }
        let _ = unsafe { SetFocus(hwnd) };
    }

    fn run_install_on_page(cfg: ProjectConfig, install_dir: String, progress: HWND, status: HWND) {
        let _ = unsafe { SendMessageW(progress, PBM_SETRANGE32, 0, 100) };
        let report_status = status;
        let report_progress = progress;
        let outcome = perform_install(&cfg, &install_dir, |percent, text| {
            let _ = unsafe { SendMessageW(report_progress, PBM_SETPOS, percent as WPARAM, 0) };
            set_text(report_status, text);
        });
        match outcome {
            Ok(done) => {
                with_state(|s| {
                    s.journal = Some(done.journal);
                    s.install_dir = done.install_dir;
                    s.install_done = true;
                    s.body_full = page_body(s, Page::Installing);
                    s.typewriter = Typewriter::new(&s.body_full, s.cfg.typewriter_interval_ms);
                });
                // Refresh body + nav for the completed state.
                let body = with_state(|s| s.body_hwnd).unwrap_or(std::ptr::null_mut());
                let next = with_state(|s| s.next_hwnd).unwrap_or(std::ptr::null_mut());
                let back = with_state(|s| s.back_hwnd).unwrap_or(std::ptr::null_mut());
                let hwnd = with_state(|s| s.hwnd).unwrap_or(std::ptr::null_mut());
                set_text(body, "");
                let interval = cfg.typewriter_interval_ms.max(1);
                let _ = unsafe { SetTimer(hwnd, TIMER_TYPEWRITER, interval, None) };
                set_enabled(next, true);
                set_enabled(back, false);
            }
            Err(err) => {
                with_state(|s| {
                    s.install_error = Some(err);
                    s.body_full = page_body(s, Page::Installing);
                    s.typewriter = Typewriter::new(&s.body_full, s.cfg.typewriter_interval_ms);
                });
                let body = with_state(|s| s.body_hwnd).unwrap_or(std::ptr::null_mut());
                let back = with_state(|s| s.back_hwnd).unwrap_or(std::ptr::null_mut());
                let hwnd = with_state(|s| s.hwnd).unwrap_or(std::ptr::null_mut());
                set_text(body, "");
                let interval = cfg.typewriter_interval_ms.max(1);
                let _ = unsafe { SetTimer(hwnd, TIMER_TYPEWRITER, interval, None) };
                set_enabled(back, true);
            }
        }
    }

    fn build_finish_controls(hwnd: HWND, font: HFONT, cfg: &ProjectConfig) {
        let model = cfg.finish_model();
        let f = &cfg.finish;
        let base_style = WS_CHILD_ | WS_VISIBLE_ | WS_TABSTOP_ | 3; // BS_AUTOCHECKBOX
        let mut y = 182i32;
        let mut desktop = std::ptr::null_mut();
        let mut launch = std::ptr::null_mut();
        let mut launch_tray = std::ptr::null_mut();
        let mut login = std::ptr::null_mut();
        let mut login_tray = std::ptr::null_mut();
        unsafe {
            if model.show_desktop_shortcut {
                desktop = make_child(
                    hwnd,
                    "BUTTON",
                    "Create Desktop shortcut",
                    base_style,
                    24,
                    y,
                    480,
                    24,
                    ID_CHK_DESKTOP,
                    font,
                );
                y += 28;
            }
            if model.show_launch_after_setup {
                launch = make_child(
                    hwnd,
                    "BUTTON",
                    "Launch after setup exits",
                    base_style,
                    24,
                    y,
                    480,
                    24,
                    ID_CHK_LAUNCH,
                    font,
                );
                y += 28;
            }
            if model.show_launch_tray {
                launch_tray = make_child(
                    hwnd,
                    "BUTTON",
                    "Start minimized to system tray",
                    base_style,
                    48,
                    y,
                    456,
                    24,
                    ID_CHK_LAUNCH_TRAY,
                    font,
                );
                y += 28;
            }
            if model.show_launch_at_login {
                login = make_child(
                    hwnd,
                    "BUTTON",
                    "Launch at Windows login",
                    base_style,
                    24,
                    y,
                    480,
                    24,
                    ID_CHK_LOGIN,
                    font,
                );
                y += 28;
            }
            if model.show_login_tray {
                login_tray = make_child(
                    hwnd,
                    "BUTTON",
                    "Start minimized to system tray",
                    base_style,
                    48,
                    y,
                    456,
                    24,
                    ID_CHK_LOGIN_TRAY,
                    font,
                );
            }
        }
        let _ = y;
        set_checked(desktop, f.desktop_shortcut_default);
        set_checked(launch, f.launch_after_setup_default);
        set_checked(launch_tray, f.launch_tray_default);
        set_checked(login, f.launch_at_login_default);
        set_checked(login_tray, f.login_tray_default);
        set_enabled(launch_tray, is_checked(launch));
        set_enabled(login_tray, is_checked(login));
        with_state(|s| {
            s.chk_desktop = desktop;
            s.chk_launch = launch;
            s.chk_launch_tray = launch_tray;
            s.chk_login = login;
            s.chk_login_tray = login_tray;
        });
    }

    fn on_next() {
        let Some((page, index, page_count)) =
            with_state(|s| (s.pages[s.page_index], s.page_index, s.pages.len()))
        else {
            return;
        };
        match page {
            Page::Directory => {
                let edit = with_state(|s| s.dir_edit).unwrap_or(std::ptr::null_mut());
                let chosen = get_text(edit).trim().to_string();
                if chosen.is_empty() {
                    message_box("Please choose an install folder first.", "Setup", true);
                    return;
                }
                with_state(|s| {
                    s.install_dir = chosen;
                    s.install_done = false;
                    s.install_error = None;
                    s.journal = None;
                });
                show_page(index + 1);
            }
            Page::Finish => finish_and_close(),
            _ => {
                if index + 1 < page_count {
                    show_page(index + 1);
                }
            }
        }
    }

    fn on_back() {
        let Some(index) = with_state(|s| s.page_index) else {
            return;
        };
        if index > 0 {
            show_page(index - 1);
        }
    }

    /// Finish button: apply the finish-page choices, persist them in the
    /// journal, optionally launch the app, then close the wizard.
    fn finish_and_close() {
        let Some(snapshot) = with_state(|s| {
            (
                s.cfg.clone(),
                s.install_dir.clone(),
                is_checked(s.chk_desktop),
                is_checked(s.chk_launch),
                is_checked(s.chk_launch_tray),
                is_checked(s.chk_login),
                is_checked(s.chk_login_tray),
                s.hwnd,
            )
        }) else {
            return;
        };
        let (
            cfg,
            install_dir,
            want_desktop,
            want_launch,
            want_launch_tray,
            want_login,
            want_login_tray,
            hwnd,
        ) = snapshot;
        let model = cfg.finish_model();
        let install_path = PathBuf::from(&install_dir);
        let main_exe = install_path.join(&cfg.app.main_exe);

        // The journal from the install step is authoritative; the fallback
        // only exists so a demo run still records finish-page choices. It
        // must use the same scope the install used, or uninstall would look
        // in the wrong ARP root.
        let scope_label = cfg.install.scope.resolve(is_elevated()).label();
        let mut journal = with_state(|s| s.journal.take())
            .flatten()
            .unwrap_or_else(|| {
                InstallJournal::new(
                    &cfg.app.app_id,
                    &cfg.app.name,
                    &cfg.app.version,
                    &install_dir,
                    scope_label,
                )
            });

        if want_desktop && model.show_desktop_shortcut {
            if !main_exe.is_file() {
                message_box(
                    &format!(
                        "The Desktop shortcut was skipped: {} is missing from the install folder.",
                        main_exe.display()
                    ),
                    "Setup",
                    true,
                );
            } else {
                match create_shortcut(
                    &main_exe,
                    &sanitize_folder(&cfg.app.name),
                    &install_path,
                    &cfg.app.name,
                ) {
                    Ok(link) => journal.record(JournalAction::CreateShortcut {
                        path: link.to_string_lossy().into_owned(),
                    }),
                    Err(err) => message_box(
                        &format!("The Desktop shortcut could not be created.\n\n{err}"),
                        "Setup",
                        true,
                    ),
                }
            }
        }

        // Project-file [[shortcuts]] (beyond the finish-page checkbox).
        for shortcut in &cfg.shortcuts {
            if !shortcut.location_is_supported() {
                message_box(
                    &format!(
                        "Skipped shortcut '{}': location '{}' is not supported yet \
                         (use \"desktop\" or \"start-menu\").",
                        shortcut.name, shortcut.location
                    ),
                    "Setup",
                    true,
                );
                continue;
            }
            let target = install_path.join(shortcut.target.trim());
            if !target.is_file() {
                message_box(
                    &format!(
                        "Skipped shortcut '{}': target '{}' is not installed.",
                        shortcut.name, shortcut.target
                    ),
                    "Setup",
                    true,
                );
                continue;
            }
            match create_shortcut_at(
                shortcut.location.trim(),
                &target,
                shortcut.name.trim(),
                &install_path,
                &shortcut.arguments,
            ) {
                Ok(link) => journal.record(JournalAction::CreateShortcut {
                    path: link.to_string_lossy().into_owned(),
                }),
                Err(err) => message_box(
                    &format!("Shortcut '{}' could not be created.\n\n{err}", shortcut.name),
                    "Setup",
                    true,
                ),
            }
        }

        if want_login {
            if let Some(command) =
                model.login_command(&cfg, &main_exe.to_string_lossy(), true, want_login_tray)
            {
                match reg_set_sz_at(HKEY_CURRENT_USER, RUN_KEY, &cfg.app.app_id, &command) {
                    Ok(()) => journal.record_run_entry(&cfg.app.app_id),
                    Err(err) => message_box(
                        &format!("Launch at Windows login could not be enabled.\n\n{err}"),
                        "Setup",
                        true,
                    ),
                }
            }
        }

        let journal_path = install_path.join(JOURNAL_FILE_NAME);
        if let Err(e) = journal.save(&journal_path) {
            // Losing the journal here would leave shortcuts and Run
            // entries that no uninstaller can ever remove.
            message_box(
                &format!(
                    "The finish-page choices could not be recorded, so they were not applied.\n\n{e}"
                ),
                "Setup",
                true,
            );
            let _ = unsafe { DestroyWindow(hwnd) };
            return;
        }
        with_state(|s| s.journal = Some(journal));

        if want_launch && model.show_launch_after_setup {
            let args = model.launch_args(&cfg, true, want_launch_tray);
            let params = args
                .iter()
                .map(|a| {
                    if a.contains(' ') {
                        format!("\"{a}\"")
                    } else {
                        a.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let wfile = wide(&main_exe.to_string_lossy());
            let wdir = wide(&install_dir);
            if params.is_empty() {
                let _ = unsafe {
                    ShellExecuteW(
                        hwnd,
                        windows_sys::core::w!("open"),
                        pcw(&wfile),
                        std::ptr::null(),
                        pcw(&wdir),
                        SW_SHOWNORMAL,
                    )
                };
            } else {
                let wparams = wide(&params);
                let _ = unsafe {
                    ShellExecuteW(
                        hwnd,
                        windows_sys::core::w!("open"),
                        pcw(&wfile),
                        pcw(&wparams),
                        pcw(&wdir),
                        SW_SHOWNORMAL,
                    )
                };
            }
        }

        let _ = unsafe { DestroyWindow(hwnd) };
    }

    fn skip_typewriter() {
        let Some((body, visible)) = with_state(|s| {
            if s.typewriter.is_complete() {
                return None;
            }
            s.typewriter.skip();
            Some((s.body_hwnd, s.typewriter.visible_text()))
        })
        .flatten() else {
            return;
        };
        let hwnd = with_state(|s| s.hwnd).unwrap_or(std::ptr::null_mut());
        let _ = unsafe { KillTimer(hwnd, TIMER_TYPEWRITER) };
        set_text(body, &visible);
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Sent while child controls are created/destroyed, including from
        // inside our own page rebuilds: never touch state for it.
        if msg == WM_PARENTNOTIFY {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        match msg {
            WM_TIMER if wparam == TIMER_TYPEWRITER => {
                let update = with_state(|s| {
                    let interval = s.typewriter.interval_ms();
                    if !s.typewriter.advance(interval) {
                        return None;
                    }
                    Some((
                        s.body_hwnd,
                        s.typewriter.visible_text(),
                        s.typewriter.is_complete(),
                    ))
                })
                .flatten();
                if let Some((body, visible, complete)) = update {
                    set_text(body, &visible);
                    if complete {
                        let _ = unsafe { KillTimer(hwnd, TIMER_TYPEWRITER) };
                    }
                }
                0
            }
            WM_LBUTTONDOWN => {
                skip_typewriter();
                0
            }
            WM_KEYDOWN if wparam == VK_SPACE as usize || wparam == VK_RETURN as usize => {
                // Space/Enter first finish the typewriter, then advance the
                // wizard: a keyboard-only user can complete setup without
                // ever clicking Next. (Previously the key was swallowed by
                // the main window and did nothing.)
                let (complete, page, installing) = with_state(|s| {
                    (
                        s.typewriter.is_complete(),
                        s.pages.get(s.page_index).copied(),
                        s.install_done,
                    )
                })
                .unwrap_or((true, None, false));
                if !complete {
                    skip_typewriter();
                    return 0;
                }
                match page {
                    // Enter in the folder field confirms the folder.
                    Some(Page::Directory) | Some(Page::Welcome) | Some(Page::License) => {
                        on_next();
                        0
                    }
                    Some(Page::Finish) => {
                        on_next();
                        0
                    }
                    Some(Page::Installing) if !installing => 0,
                    _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
                }
            }
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as i32;
                let notify = ((wparam >> 16) & 0xFFFF) as u32;
                if notify == BN_CLICKED {
                    match id {
                        ID_NEXT => on_next(),
                        ID_BACK => on_back(),
                        ID_CANCEL => {
                            let _ = unsafe { DestroyWindow(hwnd) };
                        }
                        ID_CHK_LAUNCH => {
                            let pair = with_state(|s| (s.chk_launch, s.chk_launch_tray));
                            if let Some((launch, tray)) = pair {
                                set_enabled(tray, is_checked(launch));
                            }
                        }
                        ID_CHK_LOGIN => {
                            let pair = with_state(|s| (s.chk_login, s.chk_login_tray));
                            if let Some((login, tray)) = pair {
                                set_enabled(tray, is_checked(login));
                            }
                        }
                        _ => {}
                    }
                } else if id == ID_BODY && notify == STN_CLICKED {
                    skip_typewriter();
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT => {
                let colors =
                    with_state_ref(|s| (s.text_color, s.bg_color, s.bg_brush, s.surface_brush));
                match colors {
                    Some((text, bg, bg_brush, surface_brush)) => {
                        let hdc = wparam as windows_sys::Win32::Graphics::Gdi::HDC;
                        let _ = unsafe { SetTextColor(hdc, text) };
                        let _ = unsafe { SetBkColor(hdc, bg) };
                        let brush = if msg == WM_CTLCOLOREDIT {
                            surface_brush
                        } else {
                            bg_brush
                        };
                        brush as LRESULT
                    }
                    None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
                }
            }
            WM_DESTROY => {
                let _ = unsafe { KillTimer(hwnd, TIMER_TYPEWRITER) };
                let gdi = with_state(|s| {
                    let taken = (s.font, s.title_font, s.bg_brush, s.surface_brush);
                    s.font = std::ptr::null_mut();
                    s.title_font = std::ptr::null_mut();
                    s.bg_brush = std::ptr::null_mut();
                    s.surface_brush = std::ptr::null_mut();
                    taken
                });
                if let Some((font, title_font, bg_brush, surface_brush)) = gdi {
                    for obj in [
                        font as *mut c_void,
                        title_font as *mut c_void,
                        bg_brush as *mut c_void,
                        surface_brush as *mut c_void,
                    ] {
                        if !obj.is_null() {
                            let _ = unsafe { DeleteObject(obj) };
                        }
                    }
                }
                unsafe { PostQuitMessage(0) };
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    const USAGE: &str = "RustPacker installer\r\n\r\n\
        (no arguments)\tRun the setup wizard.\r\n\
        --uninstall\tRemove the program using the install journal beside Uninstall.exe.\r\n\
        --uninstall --silent\tUninstall without any windows.\r\n\
        --help\tShow this message.\r\n\r\n\
        Uninstall mode also starts automatically when the file is named Uninstall.exe.";

    pub fn run() -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));
        if has("--help") || has("-h") || has("/?") {
            message_box(USAGE, "RustPacker Setup", false);
            return 0;
        }
        let exe_stem = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let uninstall_mode =
            exe_stem.eq_ignore_ascii_case("Uninstall") || has("--uninstall") || has("/uninstall");
        if uninstall_mode {
            return run_uninstall(has("--silent") || has("/silent"));
        }
        // Per-machine installs need elevation: offer a UAC re-launch
        // instead of failing at the first HKLM write. A per-user install
        // never asks for (or needs) administrator rights.
        if !is_elevated() {
            let needs_machine = match load_config() {
                Ok((cfg, _)) => {
                    cfg.install.require_admin
                        || cfg.install.scope.resolve(true).is_machine_wide()
                }
                Err(_) => false,
            };
            if needs_machine {
                let text = wide(
                    "This installer needs administrator rights to install for all users.\n\n\
                     Continue and approve the Windows prompt?",
                );
                let cap = wide("Setup");
                let answer = unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        pcw(&text),
                        pcw(&cap),
                        MB_YESNO | MB_ICONQUESTION,
                    )
                };
                if answer == IDYES && relaunch_elevated() {
                    return 0;
                }
            }
        }
        run_wizard()
    }

    fn run_wizard() -> i32 {
        // Per-Monitor DPI V2 before any window exists.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

        let (cfg, packed) = match load_config() {
            Ok(v) => v,
            Err(message) => {
                // A packed installer that cannot read its own payload must
                // refuse to run rather than install a placeholder.
                message_box(
                    &format!("{message}\n\nNo changes were made to this PC."),
                    "Installation cannot continue",
                    true,
                );
                return 3;
            }
        };
        // Running-process and prerequisite checks happen before anything
        // is written, with paged progress while the checks run.
        if let Some(blocker) = preflight_blocker(&cfg) {
            message_box(&blocker, "Setup cannot continue", true);
            return 4;
        }

        let mut pages = vec![Page::Welcome];
        if !cfg.install.license_file.trim().is_empty() {
            pages.push(Page::License);
        }
        pages.push(Page::Directory);
        pages.push(Page::Installing);
        pages.push(Page::Finish);

        let theme = cfg.theme.clone();
        let bg_color = colorref(theme.background_rgb());
        let text_color = colorref(theme.text_rgb());
        let interval = cfg.typewriter_interval_ms.max(1);

        let icex = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        let _ = unsafe { windows_sys::Win32::UI::Controls::InitCommonControlsEx(&icex) };

        let face = wide("Cascadia Mono");
        let (font, title_font, bg_brush, surface_brush) = unsafe {
            (
                CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 49, pcw(&face)),
                CreateFontW(-21, 0, 0, 0, 700, 0, 0, 0, 1, 0, 0, 5, 49, pcw(&face)),
                CreateSolidBrush(bg_color),
                CreateSolidBrush(colorref(theme.surface_rgb())),
            )
        };

        let install_dir = default_install_dir(&cfg);
        let null = std::ptr::null_mut();
        let state = WizardState {
            body_full: String::new(),
            typewriter: Typewriter::new("", interval),
            pages,
            page_index: 0,
            hwnd: null,
            title_hwnd: null,
            body_hwnd: null,
            back_hwnd: null,
            next_hwnd: null,
            cancel_hwnd: null,
            dir_edit: null,
            progress: null,
            status: null,
            chk_desktop: null,
            chk_launch: null,
            chk_launch_tray: null,
            chk_login: null,
            chk_login_tray: null,
            font,
            title_font,
            bg_brush,
            surface_brush,
            text_color,
            bg_color,
            install_dir,
            journal: None,
            install_done: false,
            install_error: None,
            cfg,
            packed,
        };
        STATE.with(|cell| *cell.borrow_mut() = Some(state));

        let hinstance: HINSTANCE = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null())
        };
        let class_name = windows_sys::core::w!("RustPackerWizard");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            hbrBackground: bg_brush,
            lpszClassName: class_name,
            ..Default::default()
        };
        if unsafe { RegisterClassExW(&wc) } == 0 {
            message_box(
                "The setup window class could not be registered.",
                "Setup",
                true,
            );
            return 1;
        }

        let app_name = with_state_ref(|s| s.cfg.app.name.clone()).unwrap_or_default();
        let wtitle = wide(&format!("{app_name} Setup"));
        // Fixed compact window: caption + system menu + minimise only,
        // and no scroll-bar styles anywhere.
        let style: u32 = 0x00C0_0000 | 0x0008_0000 | 0x0002_0000; // CAPTION | SYSMENU | MINIMIZEBOX
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name,
                pcw(&wtitle),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                576,
                460,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            message_box("The setup window could not be created.", "Setup", true);
            return 1;
        }

        let (title_hwnd, body_hwnd, back_hwnd, next_hwnd, cancel_hwnd) = unsafe {
            let title = make_child(
                hwnd,
                "STATIC",
                "",
                WS_CHILD_ | WS_VISIBLE_,
                24,
                16,
                512,
                30,
                ID_TITLE,
                title_font,
            );
            let body = make_child(
                hwnd,
                "STATIC",
                "",
                WS_CHILD_ | WS_VISIBLE_ | SS_NOTIFY,
                24,
                54,
                512,
                118,
                ID_BODY,
                font,
            );
            let back = make_child(
                hwnd,
                "BUTTON",
                "< Back",
                WS_CHILD_ | WS_VISIBLE_ | WS_TABSTOP_,
                280,
                372,
                80,
                28,
                ID_BACK,
                font,
            );
            let next = make_child(
                hwnd,
                "BUTTON",
                "Next >",
                WS_CHILD_ | WS_VISIBLE_ | WS_TABSTOP_,
                372,
                372,
                80,
                28,
                ID_NEXT,
                font,
            );
            let cancel = make_child(
                hwnd,
                "BUTTON",
                "Cancel",
                WS_CHILD_ | WS_VISIBLE_ | WS_TABSTOP_,
                464,
                372,
                80,
                28,
                ID_CANCEL,
                font,
            );
            (title, body, back, next, cancel)
        };
        with_state(|s| {
            s.hwnd = hwnd;
            s.title_hwnd = title_hwnd;
            s.body_hwnd = body_hwnd;
            s.back_hwnd = back_hwnd;
            s.next_hwnd = next_hwnd;
            s.cancel_hwnd = cancel_hwnd;
        });

        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNORMAL) };
        let _ = unsafe { UpdateWindow(hwnd) };
        show_page(0);

        let mut msg = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if result <= 0 {
                break;
            }
            let _ = unsafe { TranslateMessage(&msg) };
            let _ = unsafe { DispatchMessageW(&msg) };
        }
        STATE.with(|cell| *cell.borrow_mut() = None);
        0
    }
}
