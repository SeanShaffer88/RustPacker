//! RustPacker Builder Studio.
//!
//! A pure-Win32 (windows-sys) front end over `rustpacker-core`: edit a
//! project on the left, watch the compact terminal wizard preview update
//! live on the right, then pack a `Setup.exe` with the core packer.
//!
//! The studio is Windows-only. On any other platform `main` prints a
//! clear message and exits non-zero, so host builds still compile.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("rustpacker-gui: the RustPacker Builder Studio is Windows-only (Windows 11 x64).");
    eprintln!("It cannot open its window on this platform; build for x86_64-pc-windows-gnu or x86_64-pc-windows-msvc to run it.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(windows)]
mod win {
    use rustpacker_core::payload::{
        build_portable_zip, looks_like_pe, pack_directory, sha256_hex, sha256sums_txt,
    };
    use rustpacker_core::{
        safe_file_name, FileAssociation, FinishScreenModel, FinishSelection, InstallScope,
        Prerequisite, ProcessCheck, ProjectConfig, RegistryEntry, ShortcutConfig, TerminalTheme,
    };
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        CreateFontW, CreateSolidBrush, DeleteObject, SetBkColor, SetTextColor, UpdateWindow,
        CLEARTYPE_QUALITY, DEFAULT_CHARSET, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HFONT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
        OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    /// Icon resource id, as embedded by `build.rs`. `LoadIconW` takes a
    /// `PCWSTR`, so an integer id is cast through a pointer (windows-sys does
    /// not export the `MAKEINTRESOURCE` macro).
    const APP_ICON_ID: windows_sys::core::PCWSTR = 1 as _;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
        GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, LoadCursorW,
        LoadIconW, MessageBoxW, MoveWindow, PostQuitMessage, RegisterClassW, SendMessageW,
        SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK, BM_SETCHECK,
        BN_CLICKED, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST, CB_ADDSTRING,
        CB_GETCURSEL, CB_SETCURSEL, CREATESTRUCTW, CW_USEDEFAULT, EN_CHANGE, ES_AUTOHSCROLL,
        ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA, HMENU, IDC_ARROW, MB_ICONERROR,
        MB_OK, MINMAXINFO, MSG, SW_HIDE, SW_SHOW, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN,
        WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_GETMINMAXINFO, WM_NCCREATE, WM_SETFONT,
        WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_TABSTOP,
        WS_VISIBLE, WS_VSCROLL,
    };

    // ---- control ids -------------------------------------------------------
    const ID_TOOLBAR: i32 = 1000;
    const ID_PANEL_NAV: i32 = 1001;
    const ID_PANEL_RIGHT: i32 = 1002;
    const ID_PANEL_CENTER: i32 = 1003;
    const ID_STATUSBAR: i32 = 1004;
    const ID_BTN_NEW: i32 = 1009;
    const ID_BTN_LOAD: i32 = 1010;
    const ID_BTN_SAVE: i32 = 1011;
    const ID_BTN_VALIDATE: i32 = 1012;
    const ID_BTN_BUILD: i32 = 1013;
    const ID_BTN_TOGGLE_LEFT: i32 = 1014;
    const ID_BTN_TOGGLE_RIGHT: i32 = 1015;

    // Menu commands (also delivered through WM_COMMAND, like buttons).
    const IDM_NEW: i32 = 2001;
    const IDM_OPEN: i32 = 2002;
    const IDM_SAVE: i32 = 2003;
    const IDM_EXIT: i32 = 2004;
    const IDM_VALIDATE: i32 = 2011;
    const IDM_BUILD: i32 = 2012;
    const IDM_VIEW_NAV: i32 = 2021;
    const IDM_VIEW_PREVIEW: i32 = 2022;
    const IDM_ABOUT: i32 = 2031;

    // Navigation: group headers at 2100+group, page items at 2200+page.
    const ID_NAV_GROUP: i32 = 2100;
    const ID_NAV_ITEM: i32 = 2200;

    // Centre editor fields, grouped by page.
    const ID_ED_NAME: i32 = 3001;
    const ID_ED_VERSION: i32 = 3002;
    const ID_ED_PUBLISHER: i32 = 3003;
    const ID_ED_APPID: i32 = 3004;
    const ID_ED_MAINEXE: i32 = 3005;
    const ID_ED_DESCRIPTION: i32 = 3006;

    const ID_ED_SOURCE: i32 = 3010;
    const ID_BTN_INSPECT_PAYLOAD: i32 = 3011;
    const ID_ED_PAYLOAD_LIST: i32 = 3012;

    const ID_CB_SCOPE: i32 = 3020;
    const ID_ED_DEFAULTDIR: i32 = 3021;
    const ID_ED_LICENSE: i32 = 3022;
    const ID_BTN_BROWSE_LICENSE: i32 = 3023;
    const ID_ED_MINBUILD: i32 = 3024;
    const ID_CB_PATH: i32 = 3025;
    const ID_CHK_REQUIRE_ADMIN: i32 = 3026;

    const ID_CHK_HIDE_DESKTOP: i32 = 3030;
    const ID_CHK_HIDE_LAUNCH: i32 = 3031;
    const ID_CHK_DEF_DESKTOP: i32 = 3032;
    const ID_CHK_DEF_LAUNCH: i32 = 3033;
    const ID_CHK_DEF_LAUNCH_TRAY: i32 = 3034;
    const ID_CHK_DEF_LOGIN: i32 = 3035;
    const ID_CHK_DEF_LOGIN_TRAY: i32 = 3036;

    const ID_CHK_TRAY: i32 = 3040;
    const ID_ED_TRAYARG: i32 = 3041;
    const ID_ED_STARTUPARG: i32 = 3042;

    const ID_ED_THEME_BG: i32 = 3050;
    const ID_ED_THEME_SURFACE: i32 = 3051;
    const ID_ED_THEME_ELEVATED: i32 = 3052;
    const ID_ED_THEME_BORDER: i32 = 3053;
    const ID_ED_THEME_ACCENT: i32 = 3054;
    const ID_ED_THEME_SUCCESS: i32 = 3055;
    const ID_ED_THEME_WARNING: i32 = 3056;
    const ID_ED_THEME_INFO: i32 = 3057;
    const ID_ED_THEME_TEXT: i32 = 3058;
    const ID_ED_THEME_MUTED: i32 = 3059;
    const ID_ED_THEME_CURSOR: i32 = 3060;
    const ID_ED_TYPEWRITER: i32 = 3061;
    const ID_BTN_THEME_RESET: i32 = 3062;

    const ID_ED_OUTPUT: i32 = 3070;
    const ID_ED_STUB: i32 = 3071;
    const ID_BTN_BROWSE_STUB: i32 = 3072;
    const ID_CHK_PORTABLE: i32 = 3073;
    const ID_BTN_VALIDATE2: i32 = 3074;
    const ID_BTN_BUILD2: i32 = 3075;
    const ID_ED_STATUS: i32 = 3076;

    // List editors. Each block of ten is: listbox, detail fields, Add, Remove.
    const ID_LB_SHORTCUTS: i32 = 3300;
    const ID_ED_SC_NAME: i32 = 3301;
    const ID_ED_SC_LOCATION: i32 = 3302;
    const ID_ED_SC_TARGET: i32 = 3303;
    const ID_ED_SC_ARGS: i32 = 3304;
    const ID_BTN_SC_ADD: i32 = 3305;
    const ID_BTN_SC_DEL: i32 = 3306;

    const ID_LB_REGISTRY: i32 = 3310;
    const ID_CB_REG_ROOT: i32 = 3311;
    const ID_ED_REG_KEY: i32 = 3312;
    const ID_ED_REG_NAME: i32 = 3313;
    const ID_ED_REG_VALUE: i32 = 3314;
    const ID_BTN_REG_ADD: i32 = 3315;
    const ID_BTN_REG_DEL: i32 = 3316;

    const ID_LB_ASSOC: i32 = 3320;
    const ID_ED_AS_EXT: i32 = 3321;
    const ID_ED_AS_PROGID: i32 = 3322;
    const ID_ED_AS_DESC: i32 = 3323;
    const ID_ED_AS_OPENWITH: i32 = 3324;
    const ID_BTN_AS_ADD: i32 = 3325;
    const ID_BTN_AS_DEL: i32 = 3326;

    const ID_LB_PREREQ: i32 = 3330;
    const ID_ED_PR_NAME: i32 = 3331;
    const ID_ED_PR_REGPROBE: i32 = 3332;
    const ID_ED_PR_FILEPROBE: i32 = 3333;
    const ID_ED_PR_MESSAGE: i32 = 3334;
    const ID_BTN_PR_ADD: i32 = 3335;
    const ID_BTN_PR_DEL: i32 = 3336;

    const ID_LB_PROCESS: i32 = 3340;
    const ID_ED_PC_NAME: i32 = 3341;
    const ID_ED_PC_MESSAGE: i32 = 3342;
    const ID_BTN_PC_ADD: i32 = 3343;
    const ID_BTN_PC_DEL: i32 = 3344;

    const LIST_SHORTCUTS: usize = 0;
    const LIST_REGISTRY: usize = 1;
    const LIST_ASSOC: usize = 2;
    const LIST_PREREQ: usize = 3;
    const LIST_PROCESS: usize = 4;
    const LIST_KINDS: usize = 5;
    const LIST_IDS: [i32; LIST_KINDS] = [
        ID_LB_SHORTCUTS,
        ID_LB_REGISTRY,
        ID_LB_ASSOC,
        ID_LB_PREREQ,
        ID_LB_PROCESS,
    ];

    const ID_CB_PAGE: i32 = 1201;
    const ID_PV_PANEL: i32 = 1202;
    const ID_PV_TITLE: i32 = 1203;
    const ID_PV_BODY: i32 = 1204;
    const ID_PV_CHK_DESKTOP: i32 = 1205;
    const ID_PV_CHK_LAUNCH: i32 = 1206;
    const ID_PV_CHK_LAUNCH_TRAY: i32 = 1207;
    const ID_PV_CHK_LOGIN: i32 = 1208;
    const ID_PV_CHK_LOGIN_TRAY: i32 = 1209;
    const ID_PV_BTN_PAGE: i32 = 1210;

    const PAGE_WELCOME: usize = 0;
    const PAGE_DIRECTORY: usize = 1;
    const PAGE_FINISH: usize = 2;

    // ---- editor pages and navigation groups --------------------------------
    //
    // The centre of the window shows one editor page at a time; the left
    // navigation switches between them, grouped the way installer authors
    // think about a project: the app, how it installs, what must be true
    // before it installs, how the wizard behaves, and the build.
    const EP_APPLICATION: usize = 0;
    const EP_PAYLOAD: usize = 1;
    const EP_INSTALL: usize = 2;
    const EP_SHORTCUTS: usize = 3;
    const EP_REGISTRY: usize = 4;
    const EP_ASSOC: usize = 5;
    const EP_PREREQ: usize = 6;
    const EP_PROCESS: usize = 7;
    const EP_FINISH: usize = 8;
    const EP_TRAY: usize = 9;
    const EP_THEME: usize = 10;
    const EP_BUILD: usize = 11;
    const PAGE_COUNT: usize = 12;

    const GROUPS: &[(&str, &[usize])] = &[
        ("Project", &[EP_APPLICATION, EP_PAYLOAD]),
        (
            "Installation",
            &[EP_INSTALL, EP_SHORTCUTS, EP_REGISTRY, EP_ASSOC],
        ),
        ("Requirements", &[EP_PREREQ, EP_PROCESS]),
        ("Wizard", &[EP_FINISH, EP_TRAY, EP_THEME]),
        ("Build", &[EP_BUILD]),
    ];
    const GROUP_COUNT: usize = 5;

    const PAGE_TITLES: [&str; PAGE_COUNT] = [
        "Application",
        "Payload files",
        "Install settings",
        "Shortcuts",
        "Registry entries",
        "File associations",
        "Prerequisites",
        "Running processes",
        "Finish page",
        "Tray & startup",
        "Theme & typing",
        "Build & output",
    ];

    const PAGE_HINTS: [&str; PAGE_COUNT] = [
        "What is being installed. The App ID names the uninstall key and journal; keep it stable between releases.",
        "Every file in the source folder is packed into Setup.exe, keeping its folder structure.",
        "Where the app installs, who it installs for, and the licence shown before installing.",
        "Shortcuts the installer creates. Location is desktop or start-menu; Target is a file in the install folder.",
        "Registry values written at install time and removed again by the uninstaller.",
        "File types that open with the installed app when double-clicked.",
        "Requirements checked before installing. An entry passes when its registry value or file exists.",
        "The installer refuses to continue while one of these programs is running.",
        "Which checkboxes the wizard's finish page offers, and their starting state.",
        "Tray behaviour feeds the finish page: it unlocks the start-minimised and launch-at-login options.",
        "The installer's terminal palette (#RRGGBB) and typing speed. This themes the wizard, not the Studio.",
        "Pack the payload into a self-contained Setup.exe. The portable .zip needs no installation.",
    ];

    /// Which list editor a control id belongs to, if any.
    fn list_kind_of(id: i32) -> Option<usize> {
        match id {
            ID_LB_SHORTCUTS | ID_ED_SC_NAME | ID_ED_SC_LOCATION | ID_ED_SC_TARGET
            | ID_ED_SC_ARGS | ID_BTN_SC_ADD | ID_BTN_SC_DEL => Some(LIST_SHORTCUTS),
            ID_LB_REGISTRY | ID_CB_REG_ROOT | ID_ED_REG_KEY | ID_ED_REG_NAME | ID_ED_REG_VALUE
            | ID_BTN_REG_ADD | ID_BTN_REG_DEL => Some(LIST_REGISTRY),
            ID_LB_ASSOC | ID_ED_AS_EXT | ID_ED_AS_PROGID | ID_ED_AS_DESC | ID_ED_AS_OPENWITH
            | ID_BTN_AS_ADD | ID_BTN_AS_DEL => Some(LIST_ASSOC),
            ID_LB_PREREQ | ID_ED_PR_NAME | ID_ED_PR_REGPROBE | ID_ED_PR_FILEPROBE
            | ID_ED_PR_MESSAGE | ID_BTN_PR_ADD | ID_BTN_PR_DEL => Some(LIST_PREREQ),
            ID_LB_PROCESS | ID_ED_PC_NAME | ID_ED_PC_MESSAGE | ID_BTN_PC_ADD | ID_BTN_PC_DEL => {
                Some(LIST_PROCESS)
            }
            _ => None,
        }
    }

    /// True for centre-editor fields whose edits go straight into the config.
    fn is_form_edit(id: i32) -> bool {
        (ID_ED_NAME..=ID_ED_STATUS).contains(&id) && list_kind_of(id).is_none()
    }

    /// True for checkboxes whose state goes straight into the config.
    fn is_form_check(id: i32) -> bool {
        matches!(
            id,
            ID_CHK_REQUIRE_ADMIN
                | ID_CHK_HIDE_DESKTOP
                | ID_CHK_HIDE_LAUNCH
                | ID_CHK_DEF_DESKTOP
                | ID_CHK_DEF_LAUNCH
                | ID_CHK_DEF_LAUNCH_TRAY
                | ID_CHK_DEF_LOGIN
                | ID_CHK_DEF_LOGIN_TRAY
                | ID_CHK_TRAY
                | ID_CHK_PORTABLE
        )
    }

    // ---- small helpers -----------------------------------------------------

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Startup failures happen before there is a window to report them in,
    /// so they go in a message box (L6: exiting silently left the user with
    /// a double-click that appeared to do nothing).
    unsafe fn show_startup_error(message: &str) {
        let text = wide(message);
        let caption = wide("RustPacker Builder Studio");
        let _ = MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }

    fn colorref((r, g, b): (u8, u8, u8)) -> COLORREF {
        (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
    }

    fn menu(id: i32) -> HMENU {
        id as isize as HMENU
    }

    unsafe fn get_text(hwnd: HWND) -> String {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 {
            return String::new();
        }
        buf.truncate(n as usize);
        String::from_utf16_lossy(&buf)
    }

    unsafe fn set_text(hwnd: HWND, text: &str) {
        let w = wide(&text.replace('\n', "\r\n").replace("\r\r\n", "\r\n"));
        let _ = SetWindowTextW(hwnd, w.as_ptr());
    }

    unsafe fn get_check(hwnd: HWND) -> bool {
        SendMessageW(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as LRESULT
    }

    unsafe fn set_check(hwnd: HWND, on: bool) {
        let v = if on { BST_CHECKED } else { BST_UNCHECKED };
        let _ = SendMessageW(hwnd, BM_SETCHECK, v as WPARAM, 0);
    }

    unsafe fn set_font(hwnd: HWND, font: HFONT) {
        let _ = SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
    }

    unsafe fn combo_sel(hwnd: HWND) -> i32 {
        SendMessageW(hwnd, CB_GETCURSEL, 0, 0) as i32
    }

    unsafe fn combo_set_sel(hwnd: HWND, idx: i32) {
        let _ = SendMessageW(hwnd, CB_SETCURSEL, idx as WPARAM, 0);
    }

    unsafe fn combo_add(hwnd: HWND, text: &str) {
        let w = wide(text);
        let _ = SendMessageW(hwnd, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
    }

    unsafe fn show(hwnd: HWND, visible: bool) {
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }

    unsafe fn enable(hwnd: HWND, on: bool) {
        let _ = EnableWindow(hwnd, if on { 1 } else { 0 });
    }

    unsafe fn move_to(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
        let _ = MoveWindow(hwnd, x, y, w.max(0), h.max(0), 1);
    }

    unsafe fn show_info(owner: HWND, text: &str, caption: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION;
        let text_w = wide(text);
        let caption_w = wide(caption);
        let _ = MessageBoxW(
            owner,
            text_w.as_ptr(),
            caption_w.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }

    /// The centre editor has dozens of fields; they are found by control id
    /// rather than each taking a named field on `Ctrls`.
    unsafe fn reg(app: &mut App, id: i32, hwnd: HWND) {
        app.c.by_id.push((id, hwnd));
    }

    unsafe fn ctl(app: &App, id: i32) -> HWND {
        app.c
            .by_id
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, h)| *h)
            .unwrap_or(std::ptr::null_mut())
    }

    // Listbox wrappers (LISTBOX is a USER32 class; no common-controls init).
    unsafe fn lb_reset(hwnd: HWND) {
        use windows_sys::Win32::UI::WindowsAndMessaging::LB_RESETCONTENT;
        let _ = SendMessageW(hwnd, LB_RESETCONTENT, 0, 0);
    }

    unsafe fn lb_add(hwnd: HWND, text: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::LB_ADDSTRING;
        let w = wide(text);
        let _ = SendMessageW(hwnd, LB_ADDSTRING, 0, w.as_ptr() as LPARAM);
    }

    unsafe fn lb_replace(hwnd: HWND, idx: i32, text: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{LB_DELETESTRING, LB_INSERTSTRING};
        let w = wide(text);
        let _ = SendMessageW(hwnd, LB_DELETESTRING, idx as WPARAM, 0);
        let _ = SendMessageW(hwnd, LB_INSERTSTRING, idx as WPARAM, w.as_ptr() as LPARAM);
    }

    unsafe fn lb_sel(hwnd: HWND) -> i32 {
        use windows_sys::Win32::UI::WindowsAndMessaging::LB_GETCURSEL;
        SendMessageW(hwnd, LB_GETCURSEL, 0, 0) as i32
    }

    unsafe fn lb_set_sel(hwnd: HWND, idx: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::LB_SETCURSEL;
        let _ = SendMessageW(hwnd, LB_SETCURSEL, idx as WPARAM, 0);
    }

    // ---- state ---------------------------------------------------------------

    #[derive(Default, Clone, Copy)]
    struct Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    /// One row of a centre editor page. `layout_center` stacks the rows of
    /// the visible page; all other pages' rows are hidden.
    #[derive(Clone, Copy)]
    struct Row {
        label: HWND,
        main: HWND,
        extra: HWND,
        kind: u8,
        /// Vertical space the row advances the stack.
        height: i32,
        /// Height used for the control itself (combos need room for their
        /// drop-down even though the closed control is one line tall).
        extent: i32,
        /// Grow to fill the rest of the page (log views, file listings).
        fill: bool,
    }

    const ROW_FORM: u8 = 0; // label on the left, control (and optional small button) right
    const ROW_FULL: u8 = 1; // control spans the page width
    const ROW_SPLIT: u8 = 2; // two buttons side by side (extra may be null)
    const ROW_HEADING: u8 = 3; // section heading label only

    impl Row {
        fn new(label: HWND, main: HWND, extra: HWND, kind: u8, height: i32) -> Self {
            Self {
                label,
                main,
                extra,
                kind,
                height,
                extent: height,
                fill: false,
            }
        }
    }

    struct Ctrls {
        toolbar: HWND,
        panel_nav: HWND,
        panel_center: HWND,
        panel_right: HWND,
        statusbar: HWND,
        btn_new: HWND,
        btn_load: HWND,
        btn_save: HWND,
        btn_validate: HWND,
        btn_build: HWND,
        btn_toggle_left: HWND,
        btn_toggle_right: HWND,
        // left navigation: one header button per group, one item per page
        nav_groups: Vec<HWND>,
        nav_items: Vec<HWND>,
        // centre editor: title, hint, then the per-page rows
        lbl_center_title: HWND,
        lbl_center_hint: HWND,
        pages: Vec<Vec<Row>>,
        /// Every centre-editor control, found by id via `ctl`.
        by_id: Vec<(i32, HWND)>,
        // right panel
        lbl_page: HWND,
        cb_page: HWND,
        pv_panel: HWND,
        pv_title: HWND,
        pv_body: HWND,
        pv_desktop: HWND,
        pv_launch: HWND,
        pv_launch_tray: HWND,
        pv_login: HWND,
        pv_login_tray: HWND,
        pv_btn_page: HWND,
    }

    impl Ctrls {
        fn null() -> Self {
            Self {
                toolbar: std::ptr::null_mut(),
                panel_nav: std::ptr::null_mut(),
                panel_center: std::ptr::null_mut(),
                panel_right: std::ptr::null_mut(),
                statusbar: std::ptr::null_mut(),
                btn_new: std::ptr::null_mut(),
                btn_load: std::ptr::null_mut(),
                btn_save: std::ptr::null_mut(),
                btn_validate: std::ptr::null_mut(),
                btn_build: std::ptr::null_mut(),
                btn_toggle_left: std::ptr::null_mut(),
                btn_toggle_right: std::ptr::null_mut(),
                nav_groups: Vec::new(),
                nav_items: vec![std::ptr::null_mut(); PAGE_COUNT],
                lbl_center_title: std::ptr::null_mut(),
                lbl_center_hint: std::ptr::null_mut(),
                pages: vec![Vec::new(); PAGE_COUNT],
                by_id: Vec::new(),
                lbl_page: std::ptr::null_mut(),
                cb_page: std::ptr::null_mut(),
                pv_panel: std::ptr::null_mut(),
                pv_title: std::ptr::null_mut(),
                pv_body: std::ptr::null_mut(),
                pv_desktop: std::ptr::null_mut(),
                pv_launch: std::ptr::null_mut(),
                pv_launch_tray: std::ptr::null_mut(),
                pv_login: std::ptr::null_mut(),
                pv_login_tray: std::ptr::null_mut(),
                pv_btn_page: std::ptr::null_mut(),
            }
        }
    }

    struct App {
        cfg: ProjectConfig,
        output_dir: String,
        stub_path: String,
        /// M14: build the portable ZIP alongside `Setup.exe`, like the CLI's
        /// `--portable`. A build option rather than project data, so it is
        /// not saved into the project file.
        build_portable: bool,
        project_path: Option<PathBuf>,
        /// The wizard page the preview is showing (Welcome/Directory/Finish).
        page: usize,
        /// The editor page the centre of the window is showing.
        editor_page: usize,
        /// Which navigation groups are collapsed.
        nav_collapsed: [bool; GROUP_COUNT],
        /// Selected row of each list editor (-1 = none).
        list_sel: [i32; LIST_KINDS],
        // preview finish-page checkbox states (seeded from finish defaults
        // whenever the finish model changes)
        /// The finish-page checkbox state. The re-seed rule lives in core
        /// (`FinishSelection::reseed`) so it is testable and shared.
        preview: FinishSelection,
        last_model: Option<FinishScreenModel>,
        left_collapsed: bool,
        right_collapsed: bool,
        refreshing: bool,
        hinstance: HINSTANCE,
        c: Ctrls,
        geom_left: Rect,
        geom_center: Rect,
        geom_right: Rect,
        geom_pv: Rect,
        font_ui: HFONT,
        font_bold: HFONT,
        font_mono: HFONT,
        font_title: HFONT,
        brush_bg: HBRUSH,
        brush_surface: HBRUSH,
        brush_elevated: HBRUSH,
        /// The colours `brush_surface`/`brush_elevated` were created from.
        /// `WM_CTLCOLOR*` needs both the brush and a matching `SetBkColor`;
        /// keeping the pair together is what stops the two drifting apart.
        bg_surface: COLORREF,
        bg_elevated: COLORREF,
        /// The window background, for the toolbar strip.
        bg_window: COLORREF,
        col_text: COLORREF,
        col_accent: COLORREF,
        col_muted: COLORREF,
    }

    impl App {
        fn new(hinstance: HINSTANCE, theme: &TerminalTheme) -> Self {
            let (font_ui, font_mono, font_title) = unsafe { make_fonts() };
            let surface_rgb = theme.surface_rgb();
            // L6: the two grey shades were hardcoded in `color_control`, so a
            // project with a custom theme painted its preview in One Dark
            // Pro's greys. They are now derived from the theme and kept next
            // to the brushes that were built from them.
            let elevated_rgb = TerminalTheme::parse_hex(&theme.elevated_surface_hex)
                .or_else(|| {
                    // A malformed colour in a hand-edited project file falls
                    // back to the default theme's elevated surface rather than
                    // to a colour nobody chose.
                    TerminalTheme::parse_hex(&TerminalTheme::default().elevated_surface_hex)
                })
                .unwrap_or((0x2C, 0x31, 0x3A));
            Self {
                cfg: ProjectConfig::default(),
                output_dir: "dist".to_string(),
                stub_path: String::new(),
                build_portable: false,
                project_path: None,
                page: PAGE_FINISH,
                editor_page: EP_APPLICATION,
                nav_collapsed: [false; GROUP_COUNT],
                list_sel: [-1; LIST_KINDS],
                preview: FinishSelection::from_defaults(&ProjectConfig::default().finish),
                last_model: None,
                left_collapsed: false,
                right_collapsed: false,
                refreshing: false,
                hinstance,
                c: Ctrls::null(),
                geom_left: Rect::default(),
                geom_center: Rect::default(),
                geom_right: Rect::default(),
                geom_pv: Rect::default(),
                font_ui,
                font_bold: unsafe { make_font("Segoe UI", -15, true) },
                font_mono,
                font_title,
                // Assigned by run() from the window-class background brush.
                brush_bg: std::ptr::null_mut(),
                brush_surface: unsafe { CreateSolidBrush(colorref(surface_rgb)) },
                brush_elevated: unsafe { CreateSolidBrush(colorref(elevated_rgb)) },
                bg_surface: colorref(surface_rgb),
                bg_elevated: colorref(elevated_rgb),
                bg_window: colorref(theme.background_rgb()),
                col_text: colorref(theme.text_rgb()),
                col_accent: colorref(theme.accent_rgb()),
                col_muted: colorref(theme.muted_rgb()),
            }
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(self.font_ui);
                let _ = DeleteObject(self.font_bold);
                let _ = DeleteObject(self.font_mono);
                let _ = DeleteObject(self.font_title);
                let _ = DeleteObject(self.brush_surface);
                let _ = DeleteObject(self.brush_elevated);
                // brush_bg is also the window-class background; the process
                // is exiting when this runs, so it is left to the OS.
            }
        }
    }

    unsafe fn make_font(face: &str, height: i32, bold: bool) -> HFONT {
        let w = wide(face);
        CreateFontW(
            height,
            0,
            0,
            0,
            if bold {
                FW_BOLD as i32
            } else {
                FW_NORMAL as i32
            },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            w.as_ptr(),
        )
    }

    unsafe fn make_fonts() -> (HFONT, HFONT, HFONT) {
        let ui = make_font("Segoe UI", -16, false);
        let mono = make_font("Cascadia Mono", -15, false);
        let title = make_font("Cascadia Mono", -18, true);
        (ui, mono, title)
    }

    // ---- entry point ---------------------------------------------------------

    pub fn run() {
        unsafe {
            // Per-Monitor DPI V2 awareness (best effort: older hosts ignore it).
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let theme = TerminalTheme::default();
            let hinstance: HINSTANCE = GetModuleHandleW(std::ptr::null());
            let class_name = wide("RustPackerBuilderStudio");
            let brush_bg = CreateSolidBrush(colorref(theme.background_rgb()));

            let wc = WNDCLASSW {
                style: 0x0002 | 0x0001, // CS_HREDRAW | CS_VREDRAW
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                // Resource id 1, embedded by `build.rs`.
                hIcon: LoadIconW(hinstance, APP_ICON_ID),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: brush_bg,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                show_startup_error(
                    "Could not register the window class, so the Builder Studio cannot start.",
                );
                return;
            }

            let mut app = Box::new(App::new(hinstance, &theme));
            app.brush_bg = brush_bg;
            let app_ptr = Box::into_raw(app);

            let title = wide("RustPacker Builder Studio");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1340,
                820,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                app_ptr as *const core::ffi::c_void,
            );
            if hwnd.is_null() {
                let _ = Box::from_raw(app_ptr);
                show_startup_error(
                    "Could not create the main window, so the Builder Studio cannot start.",
                );
                return;
            }
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);

            let mut msg: MSG = std::mem::zeroed();
            loop {
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret <= 0 {
                    break;
                }
                if IsDialogMessageW(hwnd, &msg) != 0 {
                    continue;
                }
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
    }

    unsafe fn app_ref(hwnd: HWND) -> Option<&'static mut App> {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr == 0 {
            None
        } else {
            Some(&mut *(ptr as *mut App))
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lparam as *const CREATESTRUCTW);
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CREATE => {
                if let Some(app) = app_ref(hwnd) {
                    create_controls(app, hwnd);
                    layout(app, hwnd);
                    apply_config_to_ui(app);
                    set_text(app.c.lbl_center_title, PAGE_TITLES[app.editor_page]);
                    set_text(app.c.lbl_center_hint, PAGE_HINTS[app.editor_page]);
                    refresh_preview(app);
                    set_status(
                        app,
                        "Ready. Edit the project, then Validate or Build Setup.exe.\n\
                         Stub: place rustpacker-stub.exe next to this program, or set the Stub path field.",
                    );
                }
                0
            }
            WM_SIZE => {
                if let Some(app) = app_ref(hwnd) {
                    layout(app, hwnd);
                }
                0
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(lparam as *mut MINMAXINFO);
                mmi.ptMinTrackSize.x = 1060;
                mmi.ptMinTrackSize.y = 640;
                0
            }
            WM_COMMAND => {
                if let Some(app) = app_ref(hwnd) {
                    handle_command(app, hwnd, wparam);
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                if let Some(app) = app_ref(hwnd) {
                    color_control(app, lparam as HWND, wparam as HDC, false)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CTLCOLOREDIT | windows_sys::Win32::UI::WindowsAndMessaging::WM_CTLCOLORLISTBOX => {
                if let Some(app) = app_ref(hwnd) {
                    color_control(app, lparam as HWND, wparam as HDC, true)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_DESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let _ = Box::from_raw(ptr as *mut App);
                }
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Pick text/background colours for a child control by identity.
    unsafe fn color_control(app: &App, ctrl: HWND, hdc: HDC, is_edit: bool) -> LRESULT {
        let c = &app.c;
        if ctrl == c.pv_title {
            let _ = SetTextColor(hdc, app.col_accent);
            let _ = SetBkColor(hdc, app.bg_elevated);
            return app.brush_elevated as LRESULT;
        }
        if ctrl == c.pv_body {
            let _ = SetTextColor(hdc, app.col_text);
            let _ = SetBkColor(hdc, app.bg_surface);
            return app.brush_surface as LRESULT;
        }
        if ctrl == c.pv_panel {
            let _ = SetTextColor(hdc, app.col_text);
            let _ = SetBkColor(hdc, app.bg_elevated);
            return app.brush_elevated as LRESULT;
        }
        if ctrl == c.pv_desktop
            || ctrl == c.pv_launch
            || ctrl == c.pv_launch_tray
            || ctrl == c.pv_login
            || ctrl == c.pv_login_tray
        {
            let _ = SetTextColor(hdc, app.col_text);
            let _ = SetBkColor(hdc, app.bg_elevated);
            return app.brush_elevated as LRESULT;
        }
        if ctrl == c.toolbar || ctrl == c.pv_btn_page {
            let _ = SetTextColor(hdc, app.col_text);
            let _ = SetBkColor(hdc, app.bg_window);
            return app.brush_bg as LRESULT;
        }
        if ctrl == c.statusbar || ctrl == c.lbl_center_hint {
            let _ = SetTextColor(hdc, app.col_muted);
            let _ = SetBkColor(hdc, app.bg_window);
            return app.brush_bg as LRESULT;
        }
        if is_edit {
            let _ = SetTextColor(hdc, app.col_text);
            let _ = SetBkColor(hdc, app.bg_elevated);
            return app.brush_elevated as LRESULT;
        }
        // Labels, panels and checkboxes sit on the surface colour.
        if ctrl == c.lbl_center_title {
            let _ = SetTextColor(hdc, app.col_accent);
        } else {
            let _ = SetTextColor(hdc, app.col_text);
        }
        let _ = SetBkColor(hdc, app.bg_surface);
        app.brush_surface as LRESULT
    }

    // ---- control creation ------------------------------------------------------

    unsafe fn mk(
        app: &App,
        parent: HWND,
        class: &str,
        text: &str,
        style: u32,
        ex_style: u32,
        id: i32,
    ) -> HWND {
        let cls = wide(class);
        let txt = wide(text);
        CreateWindowExW(
            ex_style,
            cls.as_ptr(),
            txt.as_ptr(),
            style | WS_CHILD | WS_VISIBLE,
            0,
            0,
            10,
            10,
            parent,
            menu(id),
            app.hinstance,
            std::ptr::null(),
        )
    }

    unsafe fn mk_edit(app: &App, parent: HWND, id: i32, multiline_readonly: bool) -> HWND {
        let mut style = WS_TABSTOP | WS_BORDER | (ES_AUTOHSCROLL as u32);
        if multiline_readonly {
            style |=
                (ES_MULTILINE as u32) | (ES_READONLY as u32) | (ES_AUTOVSCROLL as u32) | WS_VSCROLL;
        }
        let h = mk(app, parent, "EDIT", "", style, 0, id);
        set_font(
            h,
            if multiline_readonly && id == ID_PV_BODY {
                app.font_mono
            } else {
                app.font_ui
            },
        );
        h
    }

    unsafe fn mk_button(app: &App, parent: HWND, text: &str, id: i32) -> HWND {
        let h = mk(
            app,
            parent,
            "BUTTON",
            text,
            WS_TABSTOP | (BS_PUSHBUTTON as u32),
            0,
            id,
        );
        set_font(h, app.font_ui);
        h
    }

    unsafe fn mk_check(app: &App, parent: HWND, text: &str, id: i32, font: HFONT) -> HWND {
        let h = mk(
            app,
            parent,
            "BUTTON",
            text,
            WS_TABSTOP | (BS_AUTOCHECKBOX as u32),
            0,
            id,
        );
        set_font(h, font);
        h
    }

    unsafe fn mk_label(app: &App, parent: HWND, text: &str, id: i32) -> HWND {
        let h = mk(app, parent, "STATIC", text, 0, 0, id);
        set_font(h, app.font_ui);
        h
    }

    unsafe fn mk_combo(app: &App, parent: HWND, id: i32) -> HWND {
        let h = mk(
            app,
            parent,
            "COMBOBOX",
            "",
            WS_TABSTOP | (CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            0,
            id,
        );
        set_font(h, app.font_ui);
        h
    }

    /// An editable multi-line edit (the app description).
    unsafe fn mk_edit_multi(app: &App, parent: HWND, id: i32) -> HWND {
        let style =
            WS_TABSTOP | WS_BORDER | (ES_MULTILINE as u32) | (ES_AUTOVSCROLL as u32) | WS_VSCROLL;
        let h = mk(app, parent, "EDIT", "", style, 0, id);
        set_font(h, app.font_ui);
        h
    }

    unsafe fn mk_listbox(app: &App, parent: HWND, id: i32) -> HWND {
        use windows_sys::Win32::UI::WindowsAndMessaging::LBS_NOTIFY;
        let h = mk(
            app,
            parent,
            "LISTBOX",
            "",
            WS_TABSTOP | WS_BORDER | WS_VSCROLL | (LBS_NOTIFY as u32),
            0,
            id,
        );
        set_font(h, app.font_ui);
        h
    }

    // ---- page construction ---------------------------------------------------
    //
    // Every centre control is registered by id and appended to its page's
    // row list; `layout_center` positions the rows of the visible page.

    unsafe fn row_edit(app: &mut App, parent: HWND, page: usize, label: &str, id: i32) {
        let lh = mk_label(app, parent, label, 0);
        let eh = mk_edit(app, parent, id, false);
        reg(app, id, eh);
        app.c.pages[page].push(Row::new(lh, eh, std::ptr::null_mut(), ROW_FORM, 24));
    }

    unsafe fn row_edit_browse(
        app: &mut App,
        parent: HWND,
        page: usize,
        label: &str,
        id: i32,
        btn_id: i32,
    ) {
        let lh = mk_label(app, parent, label, 0);
        let eh = mk_edit(app, parent, id, false);
        let bh = mk_button(app, parent, "...", btn_id);
        reg(app, id, eh);
        reg(app, btn_id, bh);
        app.c.pages[page].push(Row::new(lh, eh, bh, ROW_FORM, 24));
    }

    /// A form row whose control is a combo created earlier (combos are made
    /// before everything else so their drop-downs overlap later controls).
    unsafe fn row_combo(app: &mut App, parent: HWND, page: usize, label: &str, id: i32) {
        let lh = mk_label(app, parent, label, 0);
        let ch = ctl(app, id);
        let mut row = Row::new(lh, ch, std::ptr::null_mut(), ROW_FORM, 24);
        row.extent = 150;
        app.c.pages[page].push(row);
    }

    unsafe fn row_check(app: &mut App, parent: HWND, page: usize, text: &str, id: i32) {
        let ch = mk_check(app, parent, text, id, app.font_ui);
        reg(app, id, ch);
        app.c.pages[page].push(Row::new(
            std::ptr::null_mut(),
            ch,
            std::ptr::null_mut(),
            ROW_FULL,
            22,
        ));
    }

    unsafe fn row_heading(app: &mut App, parent: HWND, page: usize, text: &str) {
        let lh = mk_label(app, parent, text, 0);
        set_font(lh, app.font_bold);
        app.c.pages[page].push(Row::new(
            lh,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            ROW_HEADING,
            22,
        ));
    }

    unsafe fn row_note(app: &mut App, parent: HWND, page: usize, text: &str) {
        let lh = mk_label(app, parent, text, 0);
        app.c.pages[page].push(Row::new(
            lh,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            ROW_HEADING,
            22,
        ));
    }

    /// One list-editor page: listbox, Add/Remove buttons, then the detail
    /// fields of the selected entry. Detail edits write back live, so there
    /// is no separate Apply step to forget.
    unsafe fn build_list_page(
        app: &mut App,
        parent: HWND,
        page: usize,
        list_id: i32,
        add_id: i32,
        del_id: i32,
        add_text: &str,
        fields: &[(&str, i32)],
    ) {
        let lb = mk_listbox(app, parent, list_id);
        reg(app, list_id, lb);
        app.c.pages[page].push(Row::new(
            std::ptr::null_mut(),
            lb,
            std::ptr::null_mut(),
            ROW_FULL,
            132,
        ));
        let add = mk_button(app, parent, add_text, add_id);
        let del = mk_button(app, parent, "Remove selected", del_id);
        reg(app, add_id, add);
        reg(app, del_id, del);
        app.c.pages[page].push(Row::new(std::ptr::null_mut(), add, del, ROW_SPLIT, 26));
        row_heading(app, parent, page, "Selected entry");
        for (label, id) in fields {
            if *id == ID_CB_REG_ROOT {
                row_combo(app, parent, page, label, *id);
            } else {
                row_edit(app, parent, page, label, *id);
            }
        }
    }

    unsafe fn create_menu_bar(parent: HWND) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            AppendMenuW, CreateMenu, CreatePopupMenu, SetMenu, MF_POPUP, MF_SEPARATOR, MF_STRING,
        };
        type HMenu = windows_sys::Win32::UI::WindowsAndMessaging::HMENU;
        let bar = CreateMenu();
        let add = |m: HMenu, id: i32, text: &str| {
            let w = wide(text);
            let _ = AppendMenuW(m, MF_STRING, id as usize, w.as_ptr());
        };
        let popup = |m: HMenu, text: &str| {
            let w = wide(text);
            let _ = AppendMenuW(bar, MF_POPUP, m as usize, w.as_ptr());
        };
        let file = CreatePopupMenu();
        add(file, IDM_NEW, "New project");
        add(file, IDM_OPEN, "Open project...");
        add(file, IDM_SAVE, "Save project...");
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, std::ptr::null());
        add(file, IDM_EXIT, "Exit");
        popup(file, "File");
        let build = CreatePopupMenu();
        add(build, IDM_VALIDATE, "Validate project");
        add(build, IDM_BUILD, "Build Setup.exe");
        popup(build, "Build");
        let view = CreatePopupMenu();
        add(view, IDM_VIEW_NAV, "Navigation panel");
        add(view, IDM_VIEW_PREVIEW, "Live preview");
        popup(view, "View");
        let help = CreatePopupMenu();
        add(help, IDM_ABOUT, "About RustPacker Studio...");
        popup(help, "Help");
        let _ = SetMenu(parent, bar);
    }

    unsafe fn create_controls(app: &mut App, parent: HWND) {
        // Backgrounds first (bottom of the z-order): toolbar strip, the
        // three panels, the status bar, and the preview panel.
        app.c.toolbar = mk(app, parent, "STATIC", "", 0, 0, ID_TOOLBAR);
        app.c.panel_nav = mk(app, parent, "STATIC", "", 0, 0, ID_PANEL_NAV);
        app.c.panel_center = mk(app, parent, "STATIC", "", 0, 0, ID_PANEL_CENTER);
        app.c.panel_right = mk(app, parent, "STATIC", "", 0, 0, ID_PANEL_RIGHT);
        app.c.statusbar = mk(app, parent, "STATIC", "", 0, 0, ID_STATUSBAR);
        app.c.pv_panel = mk(app, parent, "STATIC", "", 0, 0, ID_PV_PANEL);

        // Combos next: their drop-down rectangle overlaps controls below
        // them, so they stay low in the z-order.
        app.c.cb_page = mk_combo(app, parent, ID_CB_PAGE);
        combo_add(app.c.cb_page, "Welcome");
        combo_add(app.c.cb_page, "Directory");
        combo_add(app.c.cb_page, "Finish");
        let scope = mk_combo(app, parent, ID_CB_SCOPE);
        combo_add(scope, "Per-User");
        combo_add(scope, "Per-Machine");
        combo_add(scope, "Dual-Mode");
        reg(app, ID_CB_SCOPE, scope);
        let path = mk_combo(app, parent, ID_CB_PATH);
        combo_add(path, "Do not add to PATH");
        combo_add(path, "User PATH");
        combo_add(path, "Machine PATH");
        reg(app, ID_CB_PATH, path);
        let reg_root = mk_combo(app, parent, ID_CB_REG_ROOT);
        combo_add(reg_root, "HKCU");
        combo_add(reg_root, "HKLM");
        reg(app, ID_CB_REG_ROOT, reg_root);

        // Toolbar buttons.
        app.c.btn_new = mk_button(app, parent, "New", ID_BTN_NEW);
        app.c.btn_load = mk_button(app, parent, "Open...", ID_BTN_LOAD);
        app.c.btn_save = mk_button(app, parent, "Save...", ID_BTN_SAVE);
        app.c.btn_validate = mk_button(app, parent, "Validate", ID_BTN_VALIDATE);
        app.c.btn_build = mk_button(app, parent, "Build Setup.exe", ID_BTN_BUILD);
        app.c.btn_toggle_left = mk_button(app, parent, "Hide navigation", ID_BTN_TOGGLE_LEFT);
        app.c.btn_toggle_right = mk_button(app, parent, "Hide preview", ID_BTN_TOGGLE_RIGHT);

        // Navigation: group headers collapse/expand; items switch pages.
        for (gi, (name, _)) in GROUPS.iter().enumerate() {
            let b = mk_button(app, parent, name, ID_NAV_GROUP + gi as i32);
            set_font(b, app.font_bold);
            app.c.nav_groups.push(b);
        }
        for page in 0..PAGE_COUNT {
            let b = mk_button(app, parent, PAGE_TITLES[page], ID_NAV_ITEM + page as i32);
            app.c.nav_items[page] = b;
        }

        // Centre header.
        app.c.lbl_center_title = mk_label(app, parent, "", 0);
        set_font(app.c.lbl_center_title, app.font_title);
        app.c.lbl_center_hint = mk_label(app, parent, "", 0);

        build_pages(app, parent);
        create_menu_bar(app, parent);

        // Right panel: page selector + live preview.
        app.c.lbl_page = mk_label(app, parent, "Wizard page:", 0);
        app.c.pv_title = mk_label(app, parent, "", ID_PV_TITLE);
        set_font(app.c.pv_title, app.font_title);
        app.c.pv_body = mk_edit(app, parent, ID_PV_BODY, true);
        app.c.pv_desktop = mk_check(
            app,
            parent,
            "Create Desktop shortcut",
            ID_PV_CHK_DESKTOP,
            app.font_mono,
        );
        app.c.pv_launch = mk_check(
            app,
            parent,
            "Launch after setup exits",
            ID_PV_CHK_LAUNCH,
            app.font_mono,
        );
        app.c.pv_launch_tray = mk_check(
            app,
            parent,
            "Start minimized to system tray",
            ID_PV_CHK_LAUNCH_TRAY,
            app.font_mono,
        );
        app.c.pv_login = mk_check(
            app,
            parent,
            "Launch at Windows login",
            ID_PV_CHK_LOGIN,
            app.font_mono,
        );
        app.c.pv_login_tray = mk_check(
            app,
            parent,
            "Start minimized to system tray",
            ID_PV_CHK_LOGIN_TRAY,
            app.font_mono,
        );
        app.c.pv_btn_page = mk_button(app, parent, "Finish", ID_PV_BTN_PAGE);
        set_font(app.c.pv_btn_page, app.font_mono);
    }

    unsafe fn build_pages(app: &mut App, parent: HWND) {
        // Application.
        row_edit(app, parent, EP_APPLICATION, "App name", ID_ED_NAME);
        row_edit(app, parent, EP_APPLICATION, "Version", ID_ED_VERSION);
        row_edit(app, parent, EP_APPLICATION, "Publisher", ID_ED_PUBLISHER);
        row_edit(app, parent, EP_APPLICATION, "App ID", ID_ED_APPID);
        row_edit(
            app,
            parent,
            EP_APPLICATION,
            "Main executable",
            ID_ED_MAINEXE,
        );
        {
            let lh = mk_label(app, parent, "Description", 0);
            let eh = mk_edit_multi(app, parent, ID_ED_DESCRIPTION);
            reg(app, ID_ED_DESCRIPTION, eh);
            app.c.pages[EP_APPLICATION].push(Row::new(lh, eh, std::ptr::null_mut(), ROW_FORM, 76));
        }

        // Payload.
        row_edit(app, parent, EP_PAYLOAD, "Source directory", ID_ED_SOURCE);
        {
            let b = mk_button(app, parent, "Inspect payload files", ID_BTN_INSPECT_PAYLOAD);
            reg(app, ID_BTN_INSPECT_PAYLOAD, b);
            app.c.pages[EP_PAYLOAD].push(Row::new(
                std::ptr::null_mut(),
                b,
                std::ptr::null_mut(),
                ROW_SPLIT,
                26,
            ));
            let eh = mk_edit(app, parent, ID_ED_PAYLOAD_LIST, true);
            reg(app, ID_ED_PAYLOAD_LIST, eh);
            let mut row = Row::new(
                std::ptr::null_mut(),
                eh,
                std::ptr::null_mut(),
                ROW_FULL,
                120,
            );
            row.fill = true;
            app.c.pages[EP_PAYLOAD].push(row);
        }

        // Install settings.
        row_combo(app, parent, EP_INSTALL, "Install scope", ID_CB_SCOPE);
        row_edit(
            app,
            parent,
            EP_INSTALL,
            "Default install dir",
            ID_ED_DEFAULTDIR,
        );
        row_note(
            app,
            parent,
            EP_INSTALL,
            "Tokens: {per-user-programs} and {per-machine-programs}, or a full path.",
        );
        row_edit_browse(
            app,
            parent,
            EP_INSTALL,
            "License file",
            ID_ED_LICENSE,
            ID_BTN_BROWSE_LICENSE,
        );
        row_edit(
            app,
            parent,
            EP_INSTALL,
            "Min. Windows build",
            ID_ED_MINBUILD,
        );
        row_note(
            app,
            parent,
            EP_INSTALL,
            "Windows 11 is build 22000. Use 0 for no minimum.",
        );
        row_combo(
            app,
            parent,
            EP_INSTALL,
            "Install folder on PATH",
            ID_CB_PATH,
        );
        row_check(
            app,
            parent,
            EP_INSTALL,
            "Require administrator (per-machine installs always elevate)",
            ID_CHK_REQUIRE_ADMIN,
        );

        // List editors.
        build_list_page(
            app,
            parent,
            EP_SHORTCUTS,
            ID_LB_SHORTCUTS,
            ID_BTN_SC_ADD,
            ID_BTN_SC_DEL,
            "Add shortcut",
            &[
                ("Name", ID_ED_SC_NAME),
                ("Location", ID_ED_SC_LOCATION),
                ("Target", ID_ED_SC_TARGET),
                ("Arguments", ID_ED_SC_ARGS),
            ],
        );
        build_list_page(
            app,
            parent,
            EP_REGISTRY,
            ID_LB_REGISTRY,
            ID_BTN_REG_ADD,
            ID_BTN_REG_DEL,
            "Add registry entry",
            &[
                ("Root", ID_CB_REG_ROOT),
                ("Key", ID_ED_REG_KEY),
                ("Value name", ID_ED_REG_NAME),
                ("Value", ID_ED_REG_VALUE),
            ],
        );
        build_list_page(
            app,
            parent,
            EP_ASSOC,
            ID_LB_ASSOC,
            ID_BTN_AS_ADD,
            ID_BTN_AS_DEL,
            "Add association",
            &[
                ("Extension", ID_ED_AS_EXT),
                ("ProgID", ID_ED_AS_PROGID),
                ("Description", ID_ED_AS_DESC),
                ("Open with (exe)", ID_ED_AS_OPENWITH),
            ],
        );
        build_list_page(
            app,
            parent,
            EP_PREREQ,
            ID_LB_PREREQ,
            ID_BTN_PR_ADD,
            ID_BTN_PR_DEL,
            "Add prerequisite",
            &[
                ("Name", ID_ED_PR_NAME),
                ("Registry probe", ID_ED_PR_REGPROBE),
                ("File probe", ID_ED_PR_FILEPROBE),
                ("Message if missing", ID_ED_PR_MESSAGE),
            ],
        );
        build_list_page(
            app,
            parent,
            EP_PROCESS,
            ID_LB_PROCESS,
            ID_BTN_PC_ADD,
            ID_BTN_PC_DEL,
            "Add process check",
            &[
                ("Process name", ID_ED_PC_NAME),
                ("Message if running", ID_ED_PC_MESSAGE),
            ],
        );

        // Finish page.
        row_heading(app, parent, EP_FINISH, "Offered checkboxes");
        row_check(
            app,
            parent,
            EP_FINISH,
            "Hide 'Create Desktop shortcut'",
            ID_CHK_HIDE_DESKTOP,
        );
        row_check(
            app,
            parent,
            EP_FINISH,
            "Hide 'Launch after setup exits' (and its tray sub-option)",
            ID_CHK_HIDE_LAUNCH,
        );
        row_heading(
            app,
            parent,
            EP_FINISH,
            "Starting state (the user can change these)",
        );
        row_check(
            app,
            parent,
            EP_FINISH,
            "Create Desktop shortcut",
            ID_CHK_DEF_DESKTOP,
        );
        row_check(
            app,
            parent,
            EP_FINISH,
            "Launch after setup exits",
            ID_CHK_DEF_LAUNCH,
        );
        row_check(
            app,
            parent,
            EP_FINISH,
            "Start minimized to system tray (after launch)",
            ID_CHK_DEF_LAUNCH_TRAY,
        );
        row_check(
            app,
            parent,
            EP_FINISH,
            "Launch at Windows login",
            ID_CHK_DEF_LOGIN,
        );
        row_check(
            app,
            parent,
            EP_FINISH,
            "Start minimized to system tray (at login)",
            ID_CHK_DEF_LOGIN_TRAY,
        );

        // Tray & startup.
        row_check(
            app,
            parent,
            EP_TRAY,
            "The app can minimise to the system tray",
            ID_CHK_TRAY,
        );
        row_edit(app, parent, EP_TRAY, "Tray argument", ID_ED_TRAYARG);
        row_edit(
            app,
            parent,
            EP_TRAY,
            "Startup tray argument",
            ID_ED_STARTUPARG,
        );
        row_note(
            app,
            parent,
            EP_TRAY,
            "Leave the startup argument blank to reuse the tray argument.",
        );

        // Theme & typing.
        let theme_fields: [(&str, i32); 11] = [
            ("Background", ID_ED_THEME_BG),
            ("Surface", ID_ED_THEME_SURFACE),
            ("Elevated surface", ID_ED_THEME_ELEVATED),
            ("Border", ID_ED_THEME_BORDER),
            ("Accent", ID_ED_THEME_ACCENT),
            ("Success", ID_ED_THEME_SUCCESS),
            ("Warning", ID_ED_THEME_WARNING),
            ("Info", ID_ED_THEME_INFO),
            ("Text", ID_ED_THEME_TEXT),
            ("Muted text", ID_ED_THEME_MUTED),
            ("Cursor", ID_ED_THEME_CURSOR),
        ];
        for (label, id) in theme_fields {
            row_edit(app, parent, EP_THEME, label, id);
        }
        row_edit(
            app,
            parent,
            EP_THEME,
            "Typewriter interval (ms)",
            ID_ED_TYPEWRITER,
        );
        {
            let b = mk_button(app, parent, "Reset to One Dark Pro", ID_BTN_THEME_RESET);
            reg(app, ID_BTN_THEME_RESET, b);
            app.c.pages[EP_THEME].push(Row::new(
                std::ptr::null_mut(),
                b,
                std::ptr::null_mut(),
                ROW_SPLIT,
                26,
            ));
        }

        // Build & output.
        row_edit(app, parent, EP_BUILD, "Output directory", ID_ED_OUTPUT);
        row_edit_browse(
            app,
            parent,
            EP_BUILD,
            "Stub path",
            ID_ED_STUB,
            ID_BTN_BROWSE_STUB,
        );
        row_note(
            app,
            parent,
            EP_BUILD,
            "The stub is found next to the Studio automatically; set a path only to override it.",
        );
        row_check(
            app,
            parent,
            EP_BUILD,
            "Also build the portable .zip",
            ID_CHK_PORTABLE,
        );
        {
            let v = mk_button(app, parent, "Validate", ID_BTN_VALIDATE2);
            let b = mk_button(app, parent, "Build Setup.exe", ID_BTN_BUILD2);
            reg(app, ID_BTN_VALIDATE2, v);
            reg(app, ID_BTN_BUILD2, b);
            app.c.pages[EP_BUILD].push(Row::new(std::ptr::null_mut(), v, b, ROW_SPLIT, 28));
        }
        row_heading(app, parent, EP_BUILD, "Build log");
        {
            let eh = mk_edit(app, parent, ID_ED_STATUS, true);
            reg(app, ID_ED_STATUS, eh);
            let mut row = Row::new(
                std::ptr::null_mut(),
                eh,
                std::ptr::null_mut(),
                ROW_FULL,
                120,
            );
            row.fill = true;
            app.c.pages[EP_BUILD].push(row);
        }
    }

    // ---- layout ------------------------------------------------------------------

    unsafe fn layout(app: &mut App, hwnd: HWND) {
        let mut rc: RECT = std::mem::zeroed();
        let _ = GetClientRect(hwnd, &mut rc);
        let w = (rc.right - rc.left).max(0);
        let h = (rc.bottom - rc.top).max(0);
        let toolbar_h = 44;
        let status_h = 24;
        let gap = 4;

        move_to(app.c.toolbar, 0, 0, w, toolbar_h);
        move_to(app.c.btn_new, 8, 8, 70, 28);
        move_to(app.c.btn_load, 84, 8, 84, 28);
        move_to(app.c.btn_save, 174, 8, 84, 28);
        move_to(app.c.btn_validate, 264, 8, 92, 28);
        move_to(app.c.btn_build, 362, 8, 132, 28);
        move_to(app.c.btn_toggle_right, w - 126, 8, 118, 28);
        move_to(app.c.btn_toggle_left, w - 268, 8, 136, 28);

        move_to(app.c.statusbar, 0, h - status_h, w, status_h);

        let cy = toolbar_h;
        let ch = (h - toolbar_h - status_h).max(0);
        let nav_w = if app.left_collapsed { 0 } else { 216 };
        let pv_w = if app.right_collapsed { 0 } else { 330 };
        let center_x = if nav_w > 0 { nav_w + gap } else { 0 };
        let center_w = (w - center_x - if pv_w > 0 { pv_w + gap } else { 0 }).max(0);
        app.geom_left = Rect {
            x: 0,
            y: cy,
            w: nav_w,
            h: ch,
        };
        app.geom_center = Rect {
            x: center_x,
            y: cy,
            w: center_w,
            h: ch,
        };
        app.geom_right = Rect {
            x: if pv_w > 0 { w - pv_w } else { w },
            y: cy,
            w: pv_w,
            h: ch,
        };

        show(app.c.panel_nav, nav_w > 0);
        show(app.c.panel_center, center_w > 0);
        show(app.c.panel_right, pv_w > 0);
        if nav_w > 0 {
            move_to(app.c.panel_nav, 0, cy, nav_w, ch);
        }
        if center_w > 0 {
            move_to(app.c.panel_center, center_x, cy, center_w, ch);
        }
        if pv_w > 0 {
            move_to(app.c.panel_right, w - pv_w, cy, pv_w, ch);
        }
        layout_nav(app);
        layout_center(app);
        layout_right(app);
    }

    /// Refresh navigation captions: group headers show their collapse
    /// state, the current page is marked and bolded.
    unsafe fn refresh_nav(app: &mut App) {
        for (gi, (name, _)) in GROUPS.iter().enumerate() {
            let arrow = if app.nav_collapsed[gi] { ">" } else { "v" };
            set_text(app.c.nav_groups[gi], &format!("{arrow}  {name}"));
        }
        for page in 0..PAGE_COUNT {
            let btn = app.c.nav_items[page];
            if page == app.editor_page {
                set_text(btn, &format!(">  {}", PAGE_TITLES[page]));
                set_font(btn, app.font_bold);
            } else {
                set_text(btn, &format!("    {}", PAGE_TITLES[page]));
                set_font(btn, app.font_ui);
            }
        }
    }

    unsafe fn layout_nav(app: &mut App) {
        let g = app.geom_left;
        if g.w <= 0 {
            for b in app.c.nav_groups.clone() {
                show(b, false);
            }
            for b in app.c.nav_items.clone() {
                show(b, false);
            }
            return;
        }
        refresh_nav(app);
        let x = g.x + 10;
        let w = (g.w - 20).max(40);
        let mut y = g.y + 10;
        for (gi, (_, pages)) in GROUPS.iter().enumerate() {
            let header = app.c.nav_groups[gi];
            show(header, true);
            move_to(header, x, y, w, 26);
            y += 30;
            for page in *pages {
                let item = app.c.nav_items[*page];
                if app.nav_collapsed[gi] {
                    show(item, false);
                } else {
                    show(item, true);
                    move_to(item, x + 14, y, (w - 14).max(40), 24);
                    y += 27;
                }
            }
            y += 6;
        }
    }

    /// Show one editor page in the centre and lay out its rows. Switching
    /// pages first folds the current page's fields back into the config,
    /// so nothing typed is ever lost by navigating away.
    unsafe fn show_page(app: &mut App, hwnd: HWND, page: usize) {
        if page != app.editor_page {
            sync_config_from_ui(app);
            app.editor_page = page;
        }
        set_text(app.c.lbl_center_title, PAGE_TITLES[app.editor_page]);
        set_text(app.c.lbl_center_hint, PAGE_HINTS[app.editor_page]);
        layout(app, hwnd);
    }

    unsafe fn layout_center(app: &mut App) {
        let g = app.geom_center;
        let all: Vec<HWND> = app
            .c
            .pages
            .iter()
            .flat_map(|rows| {
                rows.iter()
                    .flat_map(|r| [r.label, r.main, r.extra])
                    .collect::<Vec<HWND>>()
            })
            .collect();
        if g.w <= 0 {
            show(app.c.lbl_center_title, false);
            show(app.c.lbl_center_hint, false);
            for h in all {
                if !h.is_null() {
                    show(h, false);
                }
            }
            return;
        }
        show(app.c.lbl_center_title, true);
        show(app.c.lbl_center_hint, true);
        move_to(app.c.lbl_center_title, g.x + 16, g.y + 10, g.w - 32, 26);
        move_to(app.c.lbl_center_hint, g.x + 16, g.y + 40, g.w - 32, 20);

        let current = app.c.pages[app.editor_page].clone();
        let current_set: Vec<HWND> = current
            .iter()
            .flat_map(|r| [r.label, r.main, r.extra])
            .collect();
        for h in all {
            if !h.is_null() && !current_set.contains(&h) {
                show(h, false);
            }
        }

        let x = g.x + 16;
        let w = (g.w - 32).max(60);
        let bottom = g.y + g.h - 12;
        let mut y = g.y + 68;
        for row in &current {
            let height = if row.fill {
                (bottom - y).max(40)
            } else {
                row.height
            };
            for h in [row.label, row.main, row.extra] {
                if !h.is_null() {
                    show(h, true);
                }
            }
            match row.kind {
                ROW_FORM => {
                    move_to(row.label, x, y + 3, 150, 20);
                    if row.extra.is_null() {
                        move_to(row.main, x + 158, y, (w - 158).max(40), row.extent);
                    } else {
                        move_to(row.main, x + 158, y, (w - 158 - 44).max(40), row.extent);
                        move_to(row.extra, x + w - 36, y, 36, 24);
                    }
                }
                ROW_FULL => move_to(row.main, x, y, w, height),
                ROW_SPLIT => {
                    let half = ((w - 8) / 2).max(40);
                    move_to(row.main, x, y, half, row.extent);
                    if !row.extra.is_null() {
                        move_to(
                            row.extra,
                            x + half + 8,
                            y,
                            (w - half - 8).max(40),
                            row.extent,
                        );
                    }
                }
                _ => move_to(row.label, x, y + 2, w, 20),
            }
            y += height + 8;
            if y > bottom {
                break;
            }
        }
    }

    unsafe fn layout_right(app: &mut App) {
        let g = app.geom_right;
        let right_hwnds = [
            app.c.lbl_page,
            app.c.cb_page,
            app.c.pv_panel,
            app.c.pv_title,
            app.c.pv_body,
            app.c.pv_desktop,
            app.c.pv_launch,
            app.c.pv_launch_tray,
            app.c.pv_login,
            app.c.pv_login_tray,
            app.c.pv_btn_page,
        ];
        if g.w <= 0 {
            app.geom_pv = Rect::default();
            for h in right_hwnds {
                show(h, false);
            }
            return;
        }
        show(app.c.lbl_page, true);
        show(app.c.cb_page, true);
        show(app.c.pv_panel, true);
        move_to(app.c.lbl_page, g.x + 12, g.y + 11, 92, 20);
        move_to(app.c.cb_page, g.x + 108, g.y + 8, 176, 110);
        app.geom_pv = Rect {
            x: g.x + 10,
            y: g.y + 42,
            w: (g.w - 20).max(0),
            h: (g.h - 52).max(0),
        };
        let pv = app.geom_pv;
        move_to(app.c.pv_panel, pv.x, pv.y, pv.w, pv.h);
        position_preview_children(app);
    }

    /// Position the preview's inner controls and set checkbox visibility
    /// from the current finish model + selected page. This is the single
    /// authority for preview-child visibility.
    unsafe fn position_preview_children(app: &mut App) {
        let pv = app.geom_pv;
        let panel_visible = pv.w > 0 && pv.h > 0;
        let model = app.cfg.finish_model();
        let on_finish = app.page == PAGE_FINISH;
        let boxes: [(HWND, bool, bool); 5] = [
            (app.c.pv_desktop, model.show_desktop_shortcut, false),
            (app.c.pv_launch, model.show_launch_after_setup, false),
            (app.c.pv_launch_tray, model.show_launch_tray, true),
            (app.c.pv_login, model.show_launch_at_login, false),
            (app.c.pv_login_tray, model.show_login_tray, true),
        ];
        show(app.c.pv_title, panel_visible);
        show(app.c.pv_body, panel_visible);
        show(app.c.pv_btn_page, panel_visible);
        if !panel_visible {
            for (h, _, _) in boxes {
                show(h, false);
            }
            return;
        }
        move_to(app.c.pv_title, pv.x + 14, pv.y + 12, pv.w - 28, 26);
        move_to(
            app.c.pv_btn_page,
            pv.x + pv.w - 134,
            pv.y + pv.h - 42,
            120,
            30,
        );

        let visible_count = boxes.iter().filter(|(_, vis, _)| on_finish && *vis).count() as i32;
        let stack_h = visible_count * 26;
        let body_h = (pv.h - 46 - 42 - stack_h - 18).max(50);
        move_to(app.c.pv_body, pv.x + 14, pv.y + 46, pv.w - 28, body_h);

        let mut y = pv.y + 46 + body_h + 10;
        for (h, vis, indent) in boxes {
            let vis_now = on_finish && vis;
            show(h, vis_now);
            if vis_now {
                let x = pv.x + 18 + if indent { 26 } else { 0 };
                move_to(h, x, y, pv.w - 50, 24);
                y += 26;
            }
        }
    }

    // ---- config <-> UI sync -------------------------------------------------------

    unsafe fn sync_config_from_ui(app: &mut App) {
        // Hidden pages' controls keep their text, so every field can be
        // read regardless of which page is on screen. List entries are not
        // read here: their detail fields write back live into the config.
        // Everything is read into locals first so no UI borrow is alive
        // while the config is being mutated.
        let name = get_text(ctl(app, ID_ED_NAME));
        let version = get_text(ctl(app, ID_ED_VERSION));
        let publisher = get_text(ctl(app, ID_ED_PUBLISHER));
        let app_id = get_text(ctl(app, ID_ED_APPID));
        let main_exe = get_text(ctl(app, ID_ED_MAINEXE));
        let description = get_text(ctl(app, ID_ED_DESCRIPTION));
        let source_dir = get_text(ctl(app, ID_ED_SOURCE));
        let default_dir = get_text(ctl(app, ID_ED_DEFAULTDIR));
        let license_file = get_text(ctl(app, ID_ED_LICENSE));
        let min_windows_build: u32 = get_text(ctl(app, ID_ED_MINBUILD))
            .trim()
            .parse()
            .unwrap_or(0);
        let require_admin = get_check(ctl(app, ID_CHK_REQUIRE_ADMIN));
        let scope = match combo_sel(ctl(app, ID_CB_SCOPE)) {
            1 => InstallScope::PerMachine,
            2 => InstallScope::DualMode,
            _ => InstallScope::PerUser,
        };
        let path_scope = match combo_sel(ctl(app, ID_CB_PATH)) {
            1 => "user".to_string(),
            2 => "machine".to_string(),
            _ => String::new(),
        };
        let supports_tray = get_check(ctl(app, ID_CHK_TRAY));
        let tray_arg = get_text(ctl(app, ID_ED_TRAYARG));
        let startup_tray_arg = get_text(ctl(app, ID_ED_STARTUPARG));
        let hide_desktop = get_check(ctl(app, ID_CHK_HIDE_DESKTOP));
        let hide_launch = get_check(ctl(app, ID_CHK_HIDE_LAUNCH));
        let def_desktop = get_check(ctl(app, ID_CHK_DEF_DESKTOP));
        let def_launch = get_check(ctl(app, ID_CHK_DEF_LAUNCH));
        let def_launch_tray = get_check(ctl(app, ID_CHK_DEF_LAUNCH_TRAY));
        let def_login = get_check(ctl(app, ID_CHK_DEF_LOGIN));
        let def_login_tray = get_check(ctl(app, ID_CHK_DEF_LOGIN_TRAY));
        let theme_bg = get_text(ctl(app, ID_ED_THEME_BG));
        let theme_surface = get_text(ctl(app, ID_ED_THEME_SURFACE));
        let theme_elevated = get_text(ctl(app, ID_ED_THEME_ELEVATED));
        let theme_border = get_text(ctl(app, ID_ED_THEME_BORDER));
        let theme_accent = get_text(ctl(app, ID_ED_THEME_ACCENT));
        let theme_success = get_text(ctl(app, ID_ED_THEME_SUCCESS));
        let theme_warning = get_text(ctl(app, ID_ED_THEME_WARNING));
        let theme_info = get_text(ctl(app, ID_ED_THEME_INFO));
        let theme_text = get_text(ctl(app, ID_ED_THEME_TEXT));
        let theme_muted = get_text(ctl(app, ID_ED_THEME_MUTED));
        let theme_cursor = get_text(ctl(app, ID_ED_THEME_CURSOR));
        let typewriter: u32 = get_text(ctl(app, ID_ED_TYPEWRITER))
            .trim()
            .parse()
            .unwrap_or(12);
        let output_dir = get_text(ctl(app, ID_ED_OUTPUT));
        let stub_path = get_text(ctl(app, ID_ED_STUB));
        let build_portable = get_check(ctl(app, ID_CHK_PORTABLE));

        let cfg = &mut app.cfg;
        cfg.app.name = name;
        cfg.app.version = version;
        cfg.app.publisher = publisher;
        cfg.app.app_id = app_id;
        cfg.app.main_exe = main_exe;
        cfg.app.description = description;
        cfg.install.source_dir = source_dir;
        cfg.install.default_dir = default_dir;
        cfg.install.license_file = license_file;
        cfg.install.min_windows_build = min_windows_build;
        cfg.install.require_admin = require_admin;
        cfg.install.scope = scope;
        cfg.path_scope = path_scope;
        cfg.tray.supports_tray = supports_tray;
        cfg.tray.tray_arg = tray_arg;
        cfg.tray.startup_tray_arg = startup_tray_arg;
        cfg.finish.hide_desktop_shortcut = hide_desktop;
        cfg.finish.hide_launch_after_setup = hide_launch;
        cfg.finish.desktop_shortcut_default = def_desktop;
        cfg.finish.launch_after_setup_default = def_launch;
        cfg.finish.launch_tray_default = def_launch_tray;
        cfg.finish.launch_at_login_default = def_login;
        cfg.finish.login_tray_default = def_login_tray;
        cfg.theme.background_hex = theme_bg;
        cfg.theme.surface_hex = theme_surface;
        cfg.theme.elevated_surface_hex = theme_elevated;
        cfg.theme.border_hex = theme_border;
        cfg.theme.accent_hex = theme_accent;
        cfg.theme.success_hex = theme_success;
        cfg.theme.warning_hex = theme_warning;
        cfg.theme.info_hex = theme_info;
        cfg.theme.text_hex = theme_text;
        cfg.theme.muted_hex = theme_muted;
        cfg.theme.cursor_hex = theme_cursor;
        cfg.typewriter_interval_ms = typewriter;
        app.output_dir = output_dir;
        app.stub_path = stub_path;
        app.build_portable = build_portable;
    }

    unsafe fn apply_theme_to_ui(app: &mut App) {
        let t = app.cfg.theme.clone();
        set_text(ctl(app, ID_ED_THEME_BG), &t.background_hex);
        set_text(ctl(app, ID_ED_THEME_SURFACE), &t.surface_hex);
        set_text(ctl(app, ID_ED_THEME_ELEVATED), &t.elevated_surface_hex);
        set_text(ctl(app, ID_ED_THEME_BORDER), &t.border_hex);
        set_text(ctl(app, ID_ED_THEME_ACCENT), &t.accent_hex);
        set_text(ctl(app, ID_ED_THEME_SUCCESS), &t.success_hex);
        set_text(ctl(app, ID_ED_THEME_WARNING), &t.warning_hex);
        set_text(ctl(app, ID_ED_THEME_INFO), &t.info_hex);
        set_text(ctl(app, ID_ED_THEME_TEXT), &t.text_hex);
        set_text(ctl(app, ID_ED_THEME_MUTED), &t.muted_hex);
        set_text(ctl(app, ID_ED_THEME_CURSOR), &t.cursor_hex);
    }

    unsafe fn apply_config_to_ui(app: &mut App) {
        app.refreshing = true;
        let cfg = app.cfg.clone();
        set_text(ctl(app, ID_ED_NAME), &cfg.app.name);
        set_text(ctl(app, ID_ED_VERSION), &cfg.app.version);
        set_text(ctl(app, ID_ED_PUBLISHER), &cfg.app.publisher);
        set_text(ctl(app, ID_ED_APPID), &cfg.app.app_id);
        set_text(ctl(app, ID_ED_MAINEXE), &cfg.app.main_exe);
        set_text(ctl(app, ID_ED_DESCRIPTION), &cfg.app.description);
        set_text(ctl(app, ID_ED_SOURCE), &cfg.install.source_dir);
        set_text(ctl(app, ID_ED_DEFAULTDIR), &cfg.install.default_dir);
        set_text(ctl(app, ID_ED_LICENSE), &cfg.install.license_file);
        set_text(
            ctl(app, ID_ED_MINBUILD),
            &cfg.install.min_windows_build.to_string(),
        );
        set_check(ctl(app, ID_CHK_REQUIRE_ADMIN), cfg.install.require_admin);
        combo_set_sel(
            ctl(app, ID_CB_SCOPE),
            match cfg.install.scope {
                InstallScope::PerUser => 0,
                InstallScope::PerMachine => 1,
                InstallScope::DualMode => 2,
            },
        );
        combo_set_sel(
            ctl(app, ID_CB_PATH),
            match cfg.path_scope.as_str() {
                "user" => 1,
                "machine" => 2,
                _ => 0,
            },
        );
        set_check(ctl(app, ID_CHK_TRAY), cfg.tray.supports_tray);
        set_text(ctl(app, ID_ED_TRAYARG), &cfg.tray.tray_arg);
        set_text(ctl(app, ID_ED_STARTUPARG), &cfg.tray.startup_tray_arg);
        set_check(
            ctl(app, ID_CHK_HIDE_DESKTOP),
            cfg.finish.hide_desktop_shortcut,
        );
        set_check(
            ctl(app, ID_CHK_HIDE_LAUNCH),
            cfg.finish.hide_launch_after_setup,
        );
        set_check(
            ctl(app, ID_CHK_DEF_DESKTOP),
            cfg.finish.desktop_shortcut_default,
        );
        set_check(
            ctl(app, ID_CHK_DEF_LAUNCH),
            cfg.finish.launch_after_setup_default,
        );
        set_check(
            ctl(app, ID_CHK_DEF_LAUNCH_TRAY),
            cfg.finish.launch_tray_default,
        );
        set_check(
            ctl(app, ID_CHK_DEF_LOGIN),
            cfg.finish.launch_at_login_default,
        );
        set_check(
            ctl(app, ID_CHK_DEF_LOGIN_TRAY),
            cfg.finish.login_tray_default,
        );
        apply_theme_to_ui(app);
        set_text(
            ctl(app, ID_ED_TYPEWRITER),
            &cfg.typewriter_interval_ms.to_string(),
        );
        set_text(ctl(app, ID_ED_OUTPUT), &app.output_dir.clone());
        set_text(ctl(app, ID_ED_STUB), &app.stub_path.clone());
        set_check(ctl(app, ID_CHK_PORTABLE), app.build_portable);
        combo_set_sel(app.c.cb_page, app.page as i32);
        app.refreshing = false;
        refresh_all_lists(app);
    }

    unsafe fn set_status(app: &App, text: &str) {
        set_text(ctl(app, ID_ED_STATUS), text);
        let first = text.lines().next().unwrap_or("");
        set_text(app.c.statusbar, &format!("  {first}"));
    }

    // ---- list editors ---------------------------------------------------------
    //
    // Each list page is a listbox of one-line summaries over a detail form.
    // Selecting a row loads it into the form; typing in the form writes the
    // entry straight back and re-renders its summary. `refreshing` guards
    // the load direction so populating the form cannot echo into the model.

    fn dash(text: &str) -> String {
        let t = text.trim();
        if t.is_empty() {
            "(unnamed)".to_string()
        } else {
            t.to_string()
        }
    }

    fn list_len(app: &App, kind: usize) -> usize {
        match kind {
            LIST_SHORTCUTS => app.cfg.shortcuts.len(),
            LIST_REGISTRY => app.cfg.registry.len(),
            LIST_ASSOC => app.cfg.file_associations.len(),
            LIST_PREREQ => app.cfg.prerequisites.len(),
            _ => app.cfg.process_checks.len(),
        }
    }

    fn list_summary(app: &App, kind: usize, idx: usize) -> String {
        match kind {
            LIST_SHORTCUTS => {
                let s = &app.cfg.shortcuts[idx];
                let args = if s.arguments.trim().is_empty() {
                    String::new()
                } else {
                    format!("  {}", s.arguments)
                };
                format!(
                    "{}  [{}]  ->  {}{}",
                    dash(&s.name),
                    s.location,
                    s.target,
                    args
                )
            }
            LIST_REGISTRY => {
                let r = &app.cfg.registry[idx];
                format!("{}\\{}  {} = {}", r.root, r.key, dash(&r.name), r.value)
            }
            LIST_ASSOC => {
                let a = &app.cfg.file_associations[idx];
                let ext = if a.extension.starts_with('.') {
                    a.extension.clone()
                } else {
                    format!(".{}", a.extension)
                };
                format!("{}  {}  opens with {}", ext, dash(&a.prog_id), a.open_with)
            }
            LIST_PREREQ => {
                let p = &app.cfg.prerequisites[idx];
                let probe = if !p.registry_probe.trim().is_empty() {
                    p.registry_probe.as_str()
                } else {
                    p.file_probe.as_str()
                };
                format!("{}  [{}]", dash(&p.name), probe)
            }
            _ => {
                let p = &app.cfg.process_checks[idx];
                format!("{}  -  {}", dash(&p.process_name), p.message)
            }
        }
    }

    unsafe fn refresh_list(app: &mut App, kind: usize) {
        let lb = ctl(app, LIST_IDS[kind]);
        lb_reset(lb);
        for i in 0..list_len(app, kind) {
            let text = list_summary(app, kind, i);
            lb_add(lb, &text);
        }
        let len = list_len(app, kind) as i32;
        if len == 0 {
            app.list_sel[kind] = -1;
        } else {
            app.list_sel[kind] = app.list_sel[kind].clamp(0, len - 1);
            lb_set_sel(lb, app.list_sel[kind]);
        }
    }

    unsafe fn refresh_all_lists(app: &mut App) {
        for kind in 0..LIST_KINDS {
            app.list_sel[kind] = if list_len(app, kind) > 0 { 0 } else { -1 };
            refresh_list(app, kind);
            populate_list_detail(app, kind);
        }
    }

    unsafe fn populate_list_detail(app: &mut App, kind: usize) {
        app.refreshing = true;
        let idx = app.list_sel[kind];
        let valid = idx >= 0 && (idx as usize) < list_len(app, kind);
        match kind {
            LIST_SHORTCUTS => {
                let e = valid.then(|| app.cfg.shortcuts[idx as usize].clone());
                set_text(ctl(app, ID_ED_SC_NAME), e.as_ref().map_or("", |v| &v.name));
                set_text(
                    ctl(app, ID_ED_SC_LOCATION),
                    e.as_ref().map_or("", |v| &v.location),
                );
                set_text(
                    ctl(app, ID_ED_SC_TARGET),
                    e.as_ref().map_or("", |v| &v.target),
                );
                set_text(
                    ctl(app, ID_ED_SC_ARGS),
                    e.as_ref().map_or("", |v| &v.arguments),
                );
            }
            LIST_REGISTRY => {
                let e = valid.then(|| app.cfg.registry[idx as usize].clone());
                combo_set_sel(
                    ctl(app, ID_CB_REG_ROOT),
                    if e.as_ref()
                        .is_some_and(|v| v.root.eq_ignore_ascii_case("HKLM"))
                    {
                        1
                    } else {
                        0
                    },
                );
                set_text(ctl(app, ID_ED_REG_KEY), e.as_ref().map_or("", |v| &v.key));
                set_text(ctl(app, ID_ED_REG_NAME), e.as_ref().map_or("", |v| &v.name));
                set_text(
                    ctl(app, ID_ED_REG_VALUE),
                    e.as_ref().map_or("", |v| &v.value),
                );
            }
            LIST_ASSOC => {
                let e = valid.then(|| app.cfg.file_associations[idx as usize].clone());
                set_text(
                    ctl(app, ID_ED_AS_EXT),
                    e.as_ref().map_or("", |v| &v.extension),
                );
                set_text(
                    ctl(app, ID_ED_AS_PROGID),
                    e.as_ref().map_or("", |v| &v.prog_id),
                );
                set_text(
                    ctl(app, ID_ED_AS_DESC),
                    e.as_ref().map_or("", |v| &v.description),
                );
                set_text(
                    ctl(app, ID_ED_AS_OPENWITH),
                    e.as_ref().map_or("", |v| &v.open_with),
                );
            }
            LIST_PREREQ => {
                let e = valid.then(|| app.cfg.prerequisites[idx as usize].clone());
                set_text(ctl(app, ID_ED_PR_NAME), e.as_ref().map_or("", |v| &v.name));
                set_text(
                    ctl(app, ID_ED_PR_REGPROBE),
                    e.as_ref().map_or("", |v| &v.registry_probe),
                );
                set_text(
                    ctl(app, ID_ED_PR_FILEPROBE),
                    e.as_ref().map_or("", |v| &v.file_probe),
                );
                set_text(
                    ctl(app, ID_ED_PR_MESSAGE),
                    e.as_ref().map_or("", |v| &v.message),
                );
            }
            _ => {
                let e = valid.then(|| app.cfg.process_checks[idx as usize].clone());
                set_text(
                    ctl(app, ID_ED_PC_NAME),
                    e.as_ref().map_or("", |v| &v.process_name),
                );
                set_text(
                    ctl(app, ID_ED_PC_MESSAGE),
                    e.as_ref().map_or("", |v| &v.message),
                );
            }
        }
        app.refreshing = false;
    }

    /// Fold the detail form into the selected entry and re-render its
    /// listbox summary in place (selection and scroll stay put).
    unsafe fn writeback_list_detail(app: &mut App, kind: usize) {
        let idx = app.list_sel[kind];
        if idx < 0 || idx as usize >= list_len(app, kind) {
            return;
        }
        let i = idx as usize;
        // Read the form first; no UI borrow may be alive while the entry
        // is borrowed mutably.
        match kind {
            LIST_SHORTCUTS => {
                let name = get_text(ctl(app, ID_ED_SC_NAME));
                let location = get_text(ctl(app, ID_ED_SC_LOCATION));
                let target = get_text(ctl(app, ID_ED_SC_TARGET));
                let arguments = get_text(ctl(app, ID_ED_SC_ARGS));
                let e = &mut app.cfg.shortcuts[i];
                e.name = name;
                e.location = location;
                e.target = target;
                e.arguments = arguments;
            }
            LIST_REGISTRY => {
                let root = if combo_sel(ctl(app, ID_CB_REG_ROOT)) == 1 {
                    "HKLM".to_string()
                } else {
                    "HKCU".to_string()
                };
                let key = get_text(ctl(app, ID_ED_REG_KEY));
                let name = get_text(ctl(app, ID_ED_REG_NAME));
                let value = get_text(ctl(app, ID_ED_REG_VALUE));
                let e = &mut app.cfg.registry[i];
                e.root = root;
                e.key = key;
                e.name = name;
                e.value = value;
            }
            LIST_ASSOC => {
                let extension = get_text(ctl(app, ID_ED_AS_EXT));
                let prog_id = get_text(ctl(app, ID_ED_AS_PROGID));
                let description = get_text(ctl(app, ID_ED_AS_DESC));
                let open_with = get_text(ctl(app, ID_ED_AS_OPENWITH));
                let e = &mut app.cfg.file_associations[i];
                e.extension = extension;
                e.prog_id = prog_id;
                e.description = description;
                e.open_with = open_with;
            }
            LIST_PREREQ => {
                let name = get_text(ctl(app, ID_ED_PR_NAME));
                let registry_probe = get_text(ctl(app, ID_ED_PR_REGPROBE));
                let file_probe = get_text(ctl(app, ID_ED_PR_FILEPROBE));
                let message = get_text(ctl(app, ID_ED_PR_MESSAGE));
                let e = &mut app.cfg.prerequisites[i];
                e.name = name;
                e.registry_probe = registry_probe;
                e.file_probe = file_probe;
                e.message = message;
            }
            _ => {
                let process_name = get_text(ctl(app, ID_ED_PC_NAME));
                let message = get_text(ctl(app, ID_ED_PC_MESSAGE));
                let e = &mut app.cfg.process_checks[i];
                e.process_name = process_name;
                e.message = message;
            }
        }
        let text = list_summary(app, kind, i);
        let lb = ctl(app, LIST_IDS[kind]);
        lb_replace(lb, idx, &text);
        lb_set_sel(lb, idx);
    }

    unsafe fn list_add(app: &mut App, kind: usize) {
        match kind {
            LIST_SHORTCUTS => app.cfg.shortcuts.push(ShortcutConfig {
                name: "New shortcut".to_string(),
                location: "desktop".to_string(),
                target: app.cfg.app.main_exe.clone(),
                arguments: String::new(),
            }),
            LIST_REGISTRY => app.cfg.registry.push(RegistryEntry {
                root: "HKCU".to_string(),
                key: format!("Software\\{}", app.cfg.app.app_id),
                name: String::new(),
                value: String::new(),
            }),
            LIST_ASSOC => app.cfg.file_associations.push(FileAssociation {
                extension: String::new(),
                prog_id: format!("{}.File", app.cfg.app.app_id),
                description: String::new(),
                open_with: app.cfg.app.main_exe.clone(),
            }),
            LIST_PREREQ => app.cfg.prerequisites.push(Prerequisite {
                name: "New prerequisite".to_string(),
                registry_probe: String::new(),
                file_probe: String::new(),
                message: String::new(),
            }),
            _ => app.cfg.process_checks.push(ProcessCheck {
                process_name: app.cfg.app.main_exe.clone(),
                message: String::new(),
            }),
        }
        app.list_sel[kind] = (list_len(app, kind) - 1) as i32;
        refresh_list(app, kind);
        populate_list_detail(app, kind);
    }

    unsafe fn list_remove(app: &mut App, kind: usize) {
        let idx = app.list_sel[kind];
        if idx < 0 || idx as usize >= list_len(app, kind) {
            return;
        }
        match kind {
            LIST_SHORTCUTS => {
                app.cfg.shortcuts.remove(idx as usize);
            }
            LIST_REGISTRY => {
                app.cfg.registry.remove(idx as usize);
            }
            LIST_ASSOC => {
                app.cfg.file_associations.remove(idx as usize);
            }
            LIST_PREREQ => {
                app.cfg.prerequisites.remove(idx as usize);
            }
            _ => {
                app.cfg.process_checks.remove(idx as usize);
            }
        }
        refresh_list(app, kind);
        populate_list_detail(app, kind);
    }

    // ---- payload inspection -----------------------------------------------------

    fn collect_payload(
        dir: &std::path::Path,
        base: &std::path::Path,
        out: &mut Vec<(String, u64)>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_payload(&path, base, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((rel.display().to_string(), size));
            }
        }
    }

    unsafe fn inspect_payload(app: &mut App) {
        sync_config_from_ui(app);
        let src = resolve_dir(app, &app.cfg.install.source_dir);
        let mut files: Vec<(String, u64)> = Vec::new();
        if src.is_dir() {
            collect_payload(&src, &src, &mut files);
        }
        files.sort();
        let total: u64 = files.iter().map(|(_, s)| *s).sum();
        let mut text = format!(
            "{} file(s), {} bytes in {}\r\n",
            files.len(),
            total,
            src.display()
        );
        if files.is_empty() {
            text.push_str("\r\nThe source directory is empty or does not exist yet.");
        }
        for (rel, size) in files.iter().take(400) {
            text.push_str(&format!("\r\n{rel}  ({size} bytes)"));
        }
        if files.len() > 400 {
            text.push_str(&format!("\r\n... and {} more", files.len() - 400));
        }
        set_text(ctl(app, ID_ED_PAYLOAD_LIST), &text);
    }

    // ---- live preview ---------------------------------------------------------------

    fn scope_label(scope: InstallScope) -> &'static str {
        // Dual-Mode is shown with the scope it actually installs as on
        // this machine (per-machine only when the installer can elevate).
        match scope.resolve(false) {
            InstallScope::PerMachine => "Per-Machine",
            _ => "Per-User",
        }
    }

    fn display_install_dir(cfg: &ProjectConfig) -> String {
        match cfg.install.default_dir.as_str() {
            "{per-user-programs}" => format!("%LOCALAPPDATA%\\Programs\\{}", cfg.app.name),
            "{per-machine-programs}" => format!("%ProgramFiles%\\{}", cfg.app.name),
            other => other.to_string(),
        }
    }

    fn preview_texts(app: &App, model: &FinishScreenModel) -> (String, String) {
        let cfg = &app.cfg;
        let name = &cfg.app.name;
        match app.page {
            PAGE_WELCOME => (
                format!("Welcome to the {name} Setup Wizard"),
                format!(
                    "Welcome to the {name} setup wizard.\n\n\
                     This will install {name} {} by {} on your computer.\n\n\
                     It is recommended that you close all other applications before starting Setup. \
                     This makes it possible to update relevant system files without having to reboot your computer.\n\n\
                     Text in this wizard types itself out, terminal-style. Click, or press Space / Enter, to reveal it instantly.\n\n\
                     Click Next to continue.",
                    cfg.app.version, cfg.app.publisher
                ),
            ),
            PAGE_DIRECTORY => (
                "Choose Install Location".to_string(),
                format!(
                    "Setup will install {name} in the following folder. To install in a different folder, click Browse and select another folder.\n\n\
                     Destination folder:\n    {}\n\n\
                     Install scope: {}\nExecutable: {}\n\n\
                     Click Install to begin installation.",
                    display_install_dir(cfg),
                    scope_label(cfg.install.scope),
                    cfg.app.main_exe
                ),
            ),
            _ => {
                let exe_path = format!("{}\\{}", display_install_dir(cfg), cfg.app.main_exe);
                let mut body = format!(
                    "{name} has been installed on your computer.\n\nClick Finish to close Setup."
                );
                if model.show_launch_after_setup && app.preview.launch {
                    let args = model.launch_args(cfg, true, app.preview.launch_tray);
                    if args.is_empty() {
                        body.push_str(&format!("\n\nOn exit: launch {}", cfg.app.main_exe));
                    } else {
                        body.push_str(&format!("\n\nOn exit: launch {} {}", cfg.app.main_exe, args.join(" ")));
                    }
                }
                if model.show_launch_at_login && app.preview.login {
                    if let Some(cmd) =
                        model.login_command(cfg, &exe_path, true, app.preview.login_tray)
                    {
                        body.push_str(&format!("\nAt Windows login: {cmd}"));
                    }
                }
                (format!("Completing the {name} Setup Wizard"), body)
            }
        }
    }

    unsafe fn refresh_preview(app: &mut App) {
        let model = app.cfg.finish_model();
        if app.last_model.as_ref() != Some(&model) {
            // L6: the model changing means an option became visible or hidden
            // — turning tray support on, or hiding the launch row. Re-seeding
            // *every* checkbox threw away the toggles the user had already
            // made to unrelated options, so the preview silently disagreed
            // with what they had just clicked. The rule now lives in core.
            app.preview = app.preview.reseed(&model, &app.cfg.finish);
            app.last_model = Some(model.clone());
        }
        let (title, body) = preview_texts(app, &model);
        set_text(app.c.pv_title, &title);
        set_text(app.c.pv_body, &body);
        set_text(
            app.c.pv_btn_page,
            match app.page {
                PAGE_WELCOME => "Next >",
                PAGE_DIRECTORY => "Install",
                _ => "Finish",
            },
        );
        set_check(app.c.pv_desktop, app.preview.desktop);
        set_check(app.c.pv_launch, app.preview.launch);
        set_check(app.c.pv_launch_tray, app.preview.launch_tray);
        set_check(app.c.pv_login, app.preview.login);
        set_check(app.c.pv_login_tray, app.preview.login_tray);
        enable(app.c.pv_launch_tray, app.preview.launch);
        enable(app.c.pv_login_tray, app.preview.login);
        position_preview_children(app);
    }

    // ---- commands ----------------------------------------------------------------------

    unsafe fn toggle_nav(app: &mut App, hwnd: HWND) {
        app.left_collapsed = !app.left_collapsed;
        set_text(
            app.c.btn_toggle_left,
            if app.left_collapsed {
                "Show navigation"
            } else {
                "Hide navigation"
            },
        );
        layout(app, hwnd);
    }

    unsafe fn toggle_preview(app: &mut App, hwnd: HWND) {
        app.right_collapsed = !app.right_collapsed;
        set_text(
            app.c.btn_toggle_right,
            if app.right_collapsed {
                "Show preview"
            } else {
                "Hide preview"
            },
        );
        layout(app, hwnd);
    }

    unsafe fn handle_command(app: &mut App, hwnd: HWND, wparam: WPARAM) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyWindow, LBN_SELCHANGE};
        let id = (wparam & 0xFFFF) as i32;
        let code = ((wparam >> 16) & 0xFFFF) as u32;
        if app.refreshing {
            return;
        }
        if code == EN_CHANGE {
            if let Some(kind) = list_kind_of(id) {
                if kind != LIST_REGISTRY || id != ID_CB_REG_ROOT {
                    writeback_list_detail(app, kind);
                }
            } else if is_form_edit(id) {
                sync_config_from_ui(app);
                refresh_preview(app);
            }
            return;
        }
        if code == CBN_SELCHANGE {
            match id {
                ID_CB_SCOPE | ID_CB_PATH => {
                    sync_config_from_ui(app);
                    refresh_preview(app);
                }
                ID_CB_REG_ROOT => writeback_list_detail(app, LIST_REGISTRY),
                ID_CB_PAGE => {
                    let sel = combo_sel(app.c.cb_page);
                    if sel >= 0 {
                        app.page = (sel as usize).min(PAGE_FINISH);
                    }
                    refresh_preview(app);
                }
                _ => {}
            }
            return;
        }
        if code == LBN_SELCHANGE {
            if let Some(kind) = list_kind_of(id) {
                app.list_sel[kind] = lb_sel(ctl(app, LIST_IDS[kind]));
                populate_list_detail(app, kind);
            }
            return;
        }
        if code != BN_CLICKED {
            return;
        }
        // Menu commands arrive with notification code 0 too, so they share
        // the button arms below.
        match id {
            ID_BTN_NEW | IDM_NEW => do_new(app),
            ID_BTN_LOAD | IDM_OPEN => do_load(app, hwnd),
            ID_BTN_SAVE | IDM_SAVE => do_save(app, hwnd),
            ID_BTN_VALIDATE | ID_BTN_VALIDATE2 | IDM_VALIDATE => do_validate(app),
            ID_BTN_BUILD | ID_BTN_BUILD2 | IDM_BUILD => do_build(app),
            IDM_EXIT => {
                let _ = DestroyWindow(hwnd);
            }
            IDM_ABOUT => show_info(
                hwnd,
                &format!(
                    "RustPacker Builder Studio {}\n\nAuthor RustPacker projects in the \
                     navigation on the left, watch the installer wizard preview on the \
                     right, and build a self-contained Setup.exe.",
                    env!("CARGO_PKG_VERSION")
                ),
                "About RustPacker Studio",
            ),
            ID_BTN_TOGGLE_LEFT | IDM_VIEW_NAV => toggle_nav(app, hwnd),
            ID_BTN_TOGGLE_RIGHT | IDM_VIEW_PREVIEW => toggle_preview(app, hwnd),
            ID_BTN_BROWSE_STUB => {
                if let Some(path) = file_dialog(
                    hwnd,
                    false,
                    "Executable files (*.exe)\0*.exe\0All files (*.*)\0*.*\0",
                    "exe",
                    "",
                ) {
                    set_text(ctl(app, ID_ED_STUB), &path.display().to_string());
                    sync_config_from_ui(app);
                }
            }
            ID_BTN_BROWSE_LICENSE => {
                if let Some(path) = file_dialog(
                    hwnd,
                    false,
                    "Text files (*.txt)\0*.txt\0All files (*.*)\0*.*\0",
                    "txt",
                    "",
                ) {
                    set_text(ctl(app, ID_ED_LICENSE), &path.display().to_string());
                    sync_config_from_ui(app);
                }
            }
            ID_BTN_INSPECT_PAYLOAD => inspect_payload(app),
            ID_BTN_THEME_RESET => {
                sync_config_from_ui(app);
                app.cfg.theme = TerminalTheme::default();
                app.refreshing = true;
                apply_theme_to_ui(app);
                app.refreshing = false;
                set_status(app, "Theme reset to the One Dark Pro palette.");
            }
            ID_BTN_SC_ADD | ID_BTN_REG_ADD | ID_BTN_AS_ADD | ID_BTN_PR_ADD | ID_BTN_PC_ADD => {
                if let Some(kind) = list_kind_of(id) {
                    list_add(app, kind);
                }
            }
            ID_BTN_SC_DEL | ID_BTN_REG_DEL | ID_BTN_AS_DEL | ID_BTN_PR_DEL | ID_BTN_PC_DEL => {
                if let Some(kind) = list_kind_of(id) {
                    list_remove(app, kind);
                }
            }
            _ if (ID_NAV_GROUP..ID_NAV_GROUP + GROUP_COUNT as i32).contains(&id) => {
                let gi = (id - ID_NAV_GROUP) as usize;
                app.nav_collapsed[gi] = !app.nav_collapsed[gi];
                layout(app, hwnd);
            }
            _ if (ID_NAV_ITEM..ID_NAV_ITEM + PAGE_COUNT as i32).contains(&id) => {
                show_page(app, hwnd, (id - ID_NAV_ITEM) as usize);
            }
            _ if is_form_check(id) => {
                sync_config_from_ui(app);
                refresh_preview(app);
            }
            ID_PV_CHK_DESKTOP
            | ID_PV_CHK_LAUNCH
            | ID_PV_CHK_LAUNCH_TRAY
            | ID_PV_CHK_LOGIN
            | ID_PV_CHK_LOGIN_TRAY => {
                app.preview.desktop = get_check(app.c.pv_desktop);
                app.preview.launch = get_check(app.c.pv_launch);
                app.preview.launch_tray = get_check(app.c.pv_launch_tray);
                app.preview.login = get_check(app.c.pv_login);
                app.preview.login_tray = get_check(app.c.pv_login_tray);
                refresh_preview(app);
            }
            ID_PV_BTN_PAGE if app.page < PAGE_FINISH => {
                app.page += 1;
                combo_set_sel(app.c.cb_page, app.page as i32);
                refresh_preview(app);
            }
            _ => {}
        }
    }

    // ---- file dialogs ---------------------------------------------------------------------

    /// Characters the file dialog will accept.
    ///
    /// L5: this was 1,024, so a deeply nested project path was silently cut
    /// short and the dialog returned a truncated path. 32,767 is the maximum
    /// a Win32 path can be; 8,192 is comfortably beyond any real project and
    /// still bounds the allocation.
    const MAX_DIALOG_PATH_CHARS: usize = 8192;

    unsafe fn file_dialog(
        owner: HWND,
        save: bool,
        filter: &str,
        def_ext: &str,
        initial_name: &str,
    ) -> Option<PathBuf> {
        let mut buf = vec![0u16; MAX_DIALOG_PATH_CHARS];
        let init: Vec<u16> = initial_name.encode_utf16().collect();
        // Leave room for the terminating NUL the dialog expects.
        let copy = init.len().min(buf.len().saturating_sub(1));
        buf[..copy].copy_from_slice(&init[..copy]);
        let filter_w = wide(filter);
        let defext_w = wide(def_ext);
        let title_w = wide(if save {
            "Save RustPacker project"
        } else {
            "Open RustPacker project"
        });
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter_w.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrDefExt = defext_w.as_ptr();
        ofn.lpstrTitle = title_w.as_ptr();
        ofn.Flags = OFN_EXPLORER
            | OFN_HIDEREADONLY
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST
            };
        let ok = if save {
            GetSaveFileNameW(&mut ofn)
        } else {
            GetOpenFileNameW(&mut ofn)
        };
        if ok == 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }

    // ---- actions ----------------------------------------------------------------------------

    unsafe fn do_new(app: &mut App) {
        app.cfg = ProjectConfig::default();
        app.output_dir = "dist".to_string();
        app.stub_path = String::new();
        app.build_portable = false;
        app.project_path = None;
        app.last_model = None;
        app.preview = FinishSelection::from_defaults(&app.cfg.finish);
        app.list_sel = [-1; LIST_KINDS];
        apply_config_to_ui(app);
        refresh_preview(app);
        set_status(
            app,
            "New project. Set the app details and payload folder, then Validate or Build.",
        );
    }

    unsafe fn do_load(app: &mut App, hwnd: HWND) {
        let Some(path) = file_dialog(
            hwnd,
            false,
            "RustPacker project (*.toml)\0*.toml\0All files (*.*)\0*.*\0",
            "toml",
            "",
        ) else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Err(e) => set_status(app, &format!("Could not read {}:\n{e}", path.display())),
            Ok(text) => match ProjectConfig::from_toml(&text) {
                Err(e) => set_status(app, &format!("Could not load project:\n{e}")),
                Ok(cfg) => {
                    // Load even when validation complains: the user needs
                    // the fields in front of them to fix the problem. The
                    // problems are shown instead of being silently kept.
                    let problems = cfg.validate();
                    app.cfg = cfg;
                    app.project_path = Some(path.clone());
                    app.last_model = None;
                    apply_config_to_ui(app);
                    refresh_preview(app);
                    let mut status = format!("Loaded project from {}", path.display());
                    if !problems.is_empty() {
                        status.push_str(&format!(
                            "\n\nThis project has {} problem(s) to fix before building:",
                            problems.len()
                        ));
                        for p in &problems {
                            status.push_str(&format!("\n- {p}"));
                        }
                    }
                    set_status(app, &status);
                }
            },
        }
    }

    unsafe fn do_save(app: &mut App, hwnd: HWND) {
        sync_config_from_ui(app);
        let toml = match app.cfg.to_toml() {
            Ok(t) => t,
            Err(e) => {
                set_status(app, &format!("Could not serialise project:\n{e}"));
                return;
            }
        };
        let initial = format!("{}.toml", safe_file_name(&app.cfg.app.app_id));
        let Some(path) = file_dialog(
            hwnd,
            true,
            "RustPacker project (*.toml)\0*.toml\0All files (*.*)\0*.*\0",
            "toml",
            &initial,
        ) else {
            return;
        };
        match std::fs::write(&path, toml.as_bytes()) {
            Ok(()) => {
                app.project_path = Some(path.clone());
                set_status(app, &format!("Saved project to {}", path.display()));
            }
            Err(e) => set_status(app, &format!("Could not write {}:\n{e}", path.display())),
        }
    }

    fn resolve_dir(app: &App, raw: &str) -> PathBuf {
        let p = PathBuf::from(raw.trim());
        if p.is_absolute() {
            return p;
        }
        if let Some(proj) = &app.project_path {
            if let Some(parent) = proj.parent() {
                return parent.join(p);
            }
        }
        p
    }

    fn validation_errors(app: &App) -> Vec<String> {
        let mut errs = app.cfg.validate();
        if app.output_dir.trim().is_empty() {
            errs.push("output directory must not be empty".to_string());
        }
        let src = resolve_dir(app, &app.cfg.install.source_dir);
        if !src.is_dir() {
            errs.push(format!("source directory not found: {}", src.display()));
        } else if !src.join(app.cfg.app.main_exe.trim()).is_file() {
            errs.push(format!(
                "app.main_exe '{}' was not found in {}",
                app.cfg.app.main_exe,
                src.display()
            ));
        }
        if !app.cfg.install.license_file.trim().is_empty() {
            let license = resolve_dir(app, &app.cfg.install.license_file);
            if !license.is_file() {
                errs.push(format!("license file not found: {}", license.display()));
            }
        }
        errs
    }

    unsafe fn do_validate(app: &mut App) {
        sync_config_from_ui(app);
        let errs = validation_errors(app);
        if errs.is_empty() {
            set_status(
                app,
                "Validation OK — no problems found. The project is ready to build.",
            );
        } else {
            let mut text = format!("Validation found {} problem(s):", errs.len());
            for e in &errs {
                text.push_str(&format!("\n- {e}"));
            }
            set_status(app, &text);
        }
    }

    fn resolve_stub(app: &App) -> Result<PathBuf, String> {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let sibling = dir.join("rustpacker-stub.exe");
                if sibling.is_file() {
                    return Ok(sibling);
                }
            }
        }
        let typed = app.stub_path.trim();
        if !typed.is_empty() {
            let p = PathBuf::from(typed);
            if p.is_file() {
                return Ok(p);
            }
            return Err(format!("stub executable not found at {typed}"));
        }
        Err("rustpacker-stub.exe was not found next to the Builder Studio, and the Stub path field is empty.".to_string())
    }

    fn build_setup(app: &App) -> Result<String, String> {
        let stub_path = resolve_stub(app)?;
        let stub = std::fs::read(&stub_path)
            .map_err(|e| format!("could not read stub {}: {e}", stub_path.display()))?;
        if !looks_like_pe(&stub) {
            return Err(format!(
                "{} is not a Windows executable (no MZ/PE headers). Select the RustPacker stub binary.",
                stub_path.display()
            ));
        }
        let source = resolve_dir(app, &app.cfg.install.source_dir);
        let config_json = app.cfg.to_json()?;
        let (exe, entries) = pack_directory(&stub, &source, &config_json)?;
        // The installer launches app.main_exe: if it is not in the payload
        // the build is broken even though packing "succeeded".
        if !entries
            .iter()
            .any(|e| e.path.eq_ignore_ascii_case(app.cfg.app.main_exe.trim()))
        {
            return Err(format!(
                "app.main_exe '{}' is not in the payload from {}.\n\nCheck the source directory.",
                app.cfg.app.main_exe,
                source.display()
            ));
        }
        let out_dir = resolve_dir(app, &app.output_dir);
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            format!(
                "could not create output directory {}: {e}",
                out_dir.display()
            )
        })?;
        // Both components use the shared sanitiser: an unvalidated
        // version string (e.g. `1.0/../../evil`) must never be able to
        // steer the output path out of the chosen folder.
        let setup_name = format!(
            "{}-v{}-Setup.exe",
            safe_file_name(&app.cfg.app.name),
            safe_file_name(&app.cfg.app.version)
        );
        let setup_path = out_dir.join(&setup_name);
        // M14: the portable ZIP comes from the same manifest as the
        // installer payload, through the same core builder the CLI's
        // `--portable` uses, so the two surfaces ship identical exports.
        let portable = if app.build_portable {
            let bytes = build_portable_zip(&app.cfg, &source, &entries)?;
            let name = format!(
                "{}-v{}-Portable.zip",
                safe_file_name(&app.cfg.app.name),
                safe_file_name(&app.cfg.app.version)
            );
            let path = out_dir.join(&name);
            Some((name, path, bytes))
        } else {
            None
        };
        std::fs::write(&setup_path, &exe)
            .map_err(|e| format!("could not write {}: {e}", setup_path.display()))?;
        if let Some((_, portable_path, portable_bytes)) = &portable {
            std::fs::write(portable_path, portable_bytes)
                .map_err(|e| format!("could not write {}: {e}", portable_path.display()))?;
        }
        // Checksums last, covering every artefact produced above: a sums
        // file that predates a failed write would vouch for bytes that are
        // no longer the ones on disk.
        let mut sum_items: Vec<(&str, &[u8])> = vec![(setup_name.as_str(), exe.as_slice())];
        if let Some((portable_name, _, portable_bytes)) = &portable {
            sum_items.push((portable_name.as_str(), portable_bytes.as_slice()));
        }
        let sums = sha256sums_txt(&sum_items);
        let sums_path = out_dir.join("SHA256SUMS.txt");
        std::fs::write(&sums_path, sums.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", sums_path.display()))?;
        let mut message = format!(
            "Build complete.\n\nSetup: {}\nFiles packed: {}\nSetup size: {} bytes\nSHA-256: {}\n\nChecksums: {}",
            setup_path.display(),
            entries.len(),
            exe.len(),
            sha256_hex(&exe),
            sums_path.display()
        );
        if let Some((_, portable_path, portable_bytes)) = &portable {
            message.push_str(&format!(
                "\n\nPortable: {}\nPortable size: {} bytes\nPortable SHA-256: {}",
                portable_path.display(),
                portable_bytes.len(),
                sha256_hex(portable_bytes)
            ));
        }
        Ok(message)
    }

    unsafe fn do_build(app: &mut App) {
        sync_config_from_ui(app);
        let errs = validation_errors(app);
        if !errs.is_empty() {
            let mut text = "Build stopped — fix these problems first:".to_string();
            for e in &errs {
                text.push_str(&format!("\n- {e}"));
            }
            set_status(app, &text);
            return;
        }
        match build_setup(app) {
            Ok(msg) => set_status(app, &msg),
            Err(e) => set_status(app, &format!("Build failed:\n{e}")),
        }
    }
}
