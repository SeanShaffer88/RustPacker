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
        extract_payload, find_payload, read_manifest, InstallJournal, InstallScope, JournalAction,
        ProjectConfig, Typewriter, JOURNAL_FILE_NAME, UNINSTALLER_FILE_NAME,
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
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, KEY_WRITE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
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
    use windows_sys::Win32::UI::Shell::{SHGetFolderPathW, ShellExecuteW, CSIDL_DESKTOPDIRECTORY};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetWindowTextW, KillTimer, LoadCursorW, MessageBoxW, PostQuitMessage, RegisterClassExW,
        SendMessageW, SetTimer, SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK,
        BM_SETCHECK, BN_CLICKED, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HMENU, IDC_ARROW,
        MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MSG, STN_CLICKED, SW_SHOWNORMAL, WM_COMMAND,
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

    /// Read the embedded project config from our own exe, if packed.
    fn load_config() -> (ProjectConfig, bool) {
        let Some(exe) = std::env::current_exe().ok() else {
            return (demo_config(), false);
        };
        let Some(bytes) = std::fs::read(&exe).ok() else {
            return (demo_config(), false);
        };
        if find_payload(&bytes).is_err() {
            return (demo_config(), false);
        }
        let Ok(manifest) = read_manifest(&bytes) else {
            return (demo_config(), false);
        };
        let Some(json) = manifest.config_json else {
            return (demo_config(), false);
        };
        match ProjectConfig::from_json(&json) {
            Ok(cfg) => (cfg, true),
            Err(_) => (demo_config(), false),
        }
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
        let per_machine = matches!(cfg.install.scope, InstallScope::PerMachine)
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

    fn desktop_dir() -> Result<PathBuf, String> {
        let mut buf = vec![0u16; 1024];
        let hr = unsafe {
            SHGetFolderPathW(
                std::ptr::null_mut(),
                CSIDL_DESKTOPDIRECTORY as i32,
                std::ptr::null_mut(),
                0,
                buf.as_mut_ptr(),
            )
        };
        if hr != 0 {
            return Err(format!(
                "could not find the Desktop folder (0x{:08x})",
                hr as u32
            ));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }

    /// Create a `.lnk` pointing at `target`. Returns the link path.
    fn create_shortcut(
        target: &Path,
        link_name: &str,
        working_dir: &Path,
        description: &str,
    ) -> Result<PathBuf, String> {
        let desktop = desktop_dir()?;
        let link_path = desktop.join(format!("{link_name}.lnk"));
        let hr_init = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        // S_OK (0) or S_FALSE (1): we own a CoUninitialize. RPC_E_CHANGED_MODE
        // means COM was already initialised differently; proceed without it.
        let we_initialised = hr_init >= 0;
        let result = create_shortcut_inner(target, &link_path, working_dir, description);
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
        let dest = PathBuf::from(install_dir);
        let own_exe =
            std::env::current_exe().map_err(|e| format!("could not locate this installer: {e}"))?;
        let per_machine = matches!(cfg.install.scope, InstallScope::PerMachine);
        let (arp_root, arp_root_name, scope_name) = if per_machine {
            (HKEY_LOCAL_MACHINE, "HKLM", "per-machine")
        } else {
            (HKEY_CURRENT_USER, "HKCU", "per-user")
        };
        let mut journal = InstallJournal::new(
            &cfg.app.app_id,
            &cfg.app.name,
            &cfg.app.version,
            install_dir,
            scope_name,
        );

        report(5, "Creating the install folder...");
        std::fs::create_dir_all(&dest)
            .map_err(|e| format!("could not create {}: {e}", dest.display()))?;
        journal.record_dir(install_dir);

        report(20, "Extracting files...");
        let exe_bytes =
            std::fs::read(&own_exe).map_err(|e| format!("could not read this installer: {e}"))?;
        let mut recorded_dirs: HashSet<String> = HashSet::new();
        recorded_dirs.insert(install_dir.to_string());
        if find_payload(&exe_bytes).is_ok() {
            let extracted = extract_payload(&exe_bytes, &dest)?;
            for rel in &extracted {
                let full = dest.join(rel.replace('/', "\\"));
                // Record any parent folders the extractor created.
                let mut parent = full.parent().map(|p| p.to_path_buf());
                while let Some(dir) = parent {
                    if dir == dest {
                        break;
                    }
                    let key = dir.to_string_lossy().into_owned();
                    if recorded_dirs.insert(key.clone()) {
                        journal.record_dir(&key);
                    }
                    parent = dir.parent().map(|p| p.to_path_buf());
                }
                journal.record_file(&full.to_string_lossy());
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

        report(92, "Saving the install journal...");
        journal.record_file(&dest.join(JOURNAL_FILE_NAME).to_string_lossy());
        journal
            .save(&dest.join(JOURNAL_FILE_NAME))
            .map_err(|e| format!("could not save the install journal: {e}"))?;

        report(100, "Installation complete.");
        Ok(InstallOutcome {
            journal,
            install_dir: install_dir.to_string(),
        })
    }

    // ------------------------------------------------------------------
    // Uninstall (journal rollback)
    // ------------------------------------------------------------------

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
        let own_key = own_exe.to_string_lossy().to_lowercase();
        let plan: Vec<JournalAction> = journal.rollback_plan().into_iter().cloned().collect();
        let mut failures: Vec<String> = Vec::new();
        for action in &plan {
            match action {
                JournalAction::CreateFile { path } | JournalAction::CreateShortcut { path } => {
                    if path.to_lowercase() == own_key {
                        // A running exe cannot delete itself; the folder
                        // cleanup below still runs and Windows leaves just
                        // this one file behind in that case.
                        continue;
                    }
                    if Path::new(path).exists() {
                        if let Err(e) = std::fs::remove_file(path) {
                            failures.push(format!("{path}: {e}"));
                        }
                    }
                }
                JournalAction::OverwriteFile { path, backup_name } => {
                    if path.to_lowercase() != own_key {
                        let _ = std::fs::remove_file(path);
                        let backup = dir.join(backup_name);
                        if backup.exists() {
                            let _ = std::fs::rename(&backup, path);
                        }
                    }
                }
                JournalAction::CreateDir { path } => {
                    // Only removes the folder when it is already empty.
                    let _ = std::fs::remove_dir(path);
                }
                JournalAction::RunEntry { name } => {
                    reg_delete_value(HKEY_CURRENT_USER, RUN_KEY, name);
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
                JournalAction::ArpEntry { root, key } => {
                    reg_delete_tree(root_hkey(root), key);
                }
                JournalAction::FileAssociation { extension, prog_id } => {
                    let classes = "Software\\Classes";
                    reg_delete_tree(HKEY_CURRENT_USER, &format!("{classes}\\{extension}"));
                    reg_delete_tree(HKEY_CURRENT_USER, &format!("{classes}\\{prog_id}"));
                }
                JournalAction::PathEntry { .. } => {
                    // This stub never writes PATH entries, so journals it
                    // creates contain none. A foreign PathEntry would need
                    // a careful PATH rewrite; leaving PATH untouched is the
                    // safe failure mode and is reported below the loop via
                    // the still-present journal copy if uninstall aborts.
                }
            }
        }
        // The journal itself and (best effort) the now-empty folder.
        let _ = std::fs::remove_file(&journal_path);
        let _ = std::fs::remove_dir(&dir);
        let _ = std::fs::remove_dir(&journal.install_dir);
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
                        "Uninstall finished, but some files could not be removed:\n\n{}",
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
            Page::License => format!(
                "Please review the license agreement before installing {}.\r\n\r\n\
                 The full agreement ships with this package as:\r\n\r\n    {}\r\n\r\n\
                 Press Next if you accept the agreement.",
                cfg.app.name, cfg.install.license_file
            ),
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

        let mut journal = with_state(|s| s.journal.take())
            .flatten()
            .unwrap_or_else(|| {
                InstallJournal::new(
                    &cfg.app.app_id,
                    &cfg.app.name,
                    &cfg.app.version,
                    &install_dir,
                    "per-user",
                )
            });

        if want_desktop && model.show_desktop_shortcut {
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
        let _ = journal.save(&journal_path);
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
                let complete = with_state_ref(|s| s.typewriter.is_complete()).unwrap_or(true);
                if complete {
                    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
                } else {
                    skip_typewriter();
                    0
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
        run_wizard()
    }

    fn run_wizard() -> i32 {
        // Per-Monitor DPI V2 before any window exists.
        let _ =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

        let (cfg, packed) = load_config();
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
