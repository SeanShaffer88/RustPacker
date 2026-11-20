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
    use rustpacker_core::payload::{pack_directory, sha256_hex, sha256sums_txt};
    use rustpacker_core::{
        safe_file_name, FinishScreenModel, FinishSelection, InstallScope, ProjectConfig,
        TerminalTheme,
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
        LoadIconW, MoveWindow, PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW,
        SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK, BM_SETCHECK, BN_CLICKED,
        BS_AUTOCHECKBOX, BS_PUSHBUTTON, CBN_SELCHANGE, CBS_DROPDOWNLIST, CB_ADDSTRING,
        CB_GETCURSEL, CB_SETCURSEL, CREATESTRUCTW, CW_USEDEFAULT, EN_CHANGE, ES_AUTOHSCROLL,
        ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA, HMENU, IDC_ARROW, MINMAXINFO,
        MSG, SW_HIDE, SW_SHOW, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
        WM_CTLCOLORSTATIC, WM_DESTROY, WM_GETMINMAXINFO, WM_NCCREATE, WM_SETFONT, WM_SIZE,
        WNDCLASSW, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_TABSTOP,
        WS_VISIBLE, WS_VSCROLL,
    };

    // ---- control ids -------------------------------------------------------
    const ID_TOOLBAR: i32 = 1000;
    const ID_PANEL_LEFT: i32 = 1001;
    const ID_PANEL_RIGHT: i32 = 1002;
    const ID_BTN_LOAD: i32 = 1010;
    const ID_BTN_SAVE: i32 = 1011;
    const ID_BTN_VALIDATE: i32 = 1012;
    const ID_BTN_BUILD: i32 = 1013;
    const ID_BTN_TOGGLE_LEFT: i32 = 1014;
    const ID_BTN_TOGGLE_RIGHT: i32 = 1015;

    const ID_ED_NAME: i32 = 1101;
    const ID_ED_VERSION: i32 = 1102;
    const ID_ED_PUBLISHER: i32 = 1103;
    const ID_ED_APPID: i32 = 1104;
    const ID_ED_MAINEXE: i32 = 1105;
    const ID_ED_SOURCE: i32 = 1106;
    const ID_ED_OUTPUT: i32 = 1107;
    const ID_CB_SCOPE: i32 = 1108;
    const ID_ED_STUB: i32 = 1109;
    const ID_BTN_BROWSE_STUB: i32 = 1110;
    const ID_ED_TRAYARG: i32 = 1111;
    const ID_ED_STARTUPARG: i32 = 1112;
    const ID_CHK_TRAY: i32 = 1113;
    const ID_CHK_HIDE_DESKTOP: i32 = 1114;
    const ID_CHK_HIDE_LAUNCH: i32 = 1115;
    const ID_ED_STATUS: i32 = 1116;

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

    // ---- small helpers -----------------------------------------------------

    /// Same structural PE check the CLI uses before packing: `MZ`, a
    /// plausible `e_lfanew`, and a `PE\0\0` signature.
    fn looks_like_pe(bytes: &[u8]) -> bool {
        if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
            return false;
        }
        let lfanew =
            u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
        if lfanew < 0x40 || lfanew.checked_add(4).map_or(true, |end| end > bytes.len()) {
            return false;
        }
        &bytes[lfanew..lfanew + 4] == b"PE\0\0"
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
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

    // ---- state ---------------------------------------------------------------

    #[derive(Default, Clone, Copy)]
    struct Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    struct Ctrls {
        toolbar: HWND,
        panel_left: HWND,
        panel_right: HWND,
        btn_load: HWND,
        btn_save: HWND,
        btn_validate: HWND,
        btn_build: HWND,
        btn_toggle_left: HWND,
        btn_toggle_right: HWND,
        // left panel labels, in row order (11 rows)
        labels: Vec<HWND>,
        ed_name: HWND,
        ed_version: HWND,
        ed_publisher: HWND,
        ed_appid: HWND,
        ed_mainexe: HWND,
        ed_source: HWND,
        ed_output: HWND,
        cb_scope: HWND,
        ed_stub: HWND,
        btn_browse_stub: HWND,
        ed_trayarg: HWND,
        ed_startuparg: HWND,
        chk_tray: HWND,
        chk_hide_desktop: HWND,
        chk_hide_launch: HWND,
        lbl_status: HWND,
        ed_status: HWND,
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
                panel_left: std::ptr::null_mut(),
                panel_right: std::ptr::null_mut(),
                btn_load: std::ptr::null_mut(),
                btn_save: std::ptr::null_mut(),
                btn_validate: std::ptr::null_mut(),
                btn_build: std::ptr::null_mut(),
                btn_toggle_left: std::ptr::null_mut(),
                btn_toggle_right: std::ptr::null_mut(),
                labels: Vec::new(),
                ed_name: std::ptr::null_mut(),
                ed_version: std::ptr::null_mut(),
                ed_publisher: std::ptr::null_mut(),
                ed_appid: std::ptr::null_mut(),
                ed_mainexe: std::ptr::null_mut(),
                ed_source: std::ptr::null_mut(),
                ed_output: std::ptr::null_mut(),
                cb_scope: std::ptr::null_mut(),
                ed_stub: std::ptr::null_mut(),
                btn_browse_stub: std::ptr::null_mut(),
                ed_trayarg: std::ptr::null_mut(),
                ed_startuparg: std::ptr::null_mut(),
                chk_tray: std::ptr::null_mut(),
                chk_hide_desktop: std::ptr::null_mut(),
                chk_hide_launch: std::ptr::null_mut(),
                lbl_status: std::ptr::null_mut(),
                ed_status: std::ptr::null_mut(),
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
        project_path: Option<PathBuf>,
        page: usize,
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
        geom_right: Rect,
        geom_pv: Rect,
        font_ui: HFONT,
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
                project_path: None,
                page: PAGE_FINISH,
                preview: FinishSelection::from_defaults(&ProjectConfig::default().finish),
                last_model: None,
                left_collapsed: false,
                right_collapsed: false,
                refreshing: false,
                hinstance,
                c: Ctrls::null(),
                geom_left: Rect::default(),
                geom_right: Rect::default(),
                geom_pv: Rect::default(),
                font_ui,
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
            let _ = RegisterClassW(&wc);

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
                1180,
                720,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                app_ptr as *const core::ffi::c_void,
            );
            if hwnd.is_null() {
                let _ = Box::from_raw(app_ptr);
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
                mmi.ptMinTrackSize.x = 840;
                mmi.ptMinTrackSize.y = 540;
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
            WM_CTLCOLOREDIT => {
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
        if is_edit {
            let _ = SetTextColor(hdc, app.col_text);
            let _ = SetBkColor(hdc, app.bg_elevated);
            return app.brush_elevated as LRESULT;
        }
        // Labels, panels and left-panel checkboxes sit on the surface colour.
        if ctrl == c.lbl_status {
            let _ = SetTextColor(hdc, app.col_muted);
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

    unsafe fn create_controls(app: &mut App, parent: HWND) {
        // Backgrounds first (bottom of the z-order): toolbar strip, the two
        // side panels, and the preview panel inside the right panel.
        app.c.toolbar = mk(app, parent, "STATIC", "", 0, 0, ID_TOOLBAR);
        app.c.panel_left = mk(app, parent, "STATIC", "", 0, 0, ID_PANEL_LEFT);
        app.c.panel_right = mk(app, parent, "STATIC", "", 0, 0, ID_PANEL_RIGHT);
        app.c.pv_panel = mk(app, parent, "STATIC", "", 0, 0, ID_PV_PANEL);

        // Combos next: their drop-down rectangle overlaps controls below
        // them, so they stay low in the z-order.
        app.c.cb_scope = mk_combo(app, parent, ID_CB_SCOPE);
        combo_add(app.c.cb_scope, "Per-User");
        combo_add(app.c.cb_scope, "Per-Machine");
        combo_add(app.c.cb_scope, "Dual-Mode");
        app.c.cb_page = mk_combo(app, parent, ID_CB_PAGE);
        combo_add(app.c.cb_page, "Welcome");
        combo_add(app.c.cb_page, "Directory");
        combo_add(app.c.cb_page, "Finish");

        // Toolbar buttons.
        app.c.btn_load = mk_button(app, parent, "Load project...", ID_BTN_LOAD);
        app.c.btn_save = mk_button(app, parent, "Save project...", ID_BTN_SAVE);
        app.c.btn_validate = mk_button(app, parent, "Validate", ID_BTN_VALIDATE);
        app.c.btn_build = mk_button(app, parent, "Build Setup.exe", ID_BTN_BUILD);
        app.c.btn_toggle_left = mk_button(app, parent, "Hide left panel", ID_BTN_TOGGLE_LEFT);
        app.c.btn_toggle_right = mk_button(app, parent, "Hide preview", ID_BTN_TOGGLE_RIGHT);

        // Left panel: labels + fields.
        let row_labels = [
            "App name",
            "Version",
            "Publisher",
            "App ID",
            "Main exe",
            "Source directory",
            "Output directory",
            "Stub path",
            "Install scope",
            "Tray arg",
            "Startup tray arg",
        ];
        for text in row_labels {
            let h = mk_label(app, parent, text, 0);
            app.c.labels.push(h);
        }
        app.c.ed_name = mk_edit(app, parent, ID_ED_NAME, false);
        app.c.ed_version = mk_edit(app, parent, ID_ED_VERSION, false);
        app.c.ed_publisher = mk_edit(app, parent, ID_ED_PUBLISHER, false);
        app.c.ed_appid = mk_edit(app, parent, ID_ED_APPID, false);
        app.c.ed_mainexe = mk_edit(app, parent, ID_ED_MAINEXE, false);
        app.c.ed_source = mk_edit(app, parent, ID_ED_SOURCE, false);
        app.c.ed_output = mk_edit(app, parent, ID_ED_OUTPUT, false);
        app.c.ed_stub = mk_edit(app, parent, ID_ED_STUB, false);
        app.c.btn_browse_stub = mk_button(app, parent, "...", ID_BTN_BROWSE_STUB);
        app.c.ed_trayarg = mk_edit(app, parent, ID_ED_TRAYARG, false);
        app.c.ed_startuparg = mk_edit(app, parent, ID_ED_STARTUPARG, false);
        app.c.chk_tray = mk_check(app, parent, "Supports tray", ID_CHK_TRAY, app.font_ui);
        app.c.chk_hide_desktop = mk_check(
            app,
            parent,
            "Hide 'Create Desktop shortcut' on finish page",
            ID_CHK_HIDE_DESKTOP,
            app.font_ui,
        );
        app.c.chk_hide_launch = mk_check(
            app,
            parent,
            "Hide 'Launch after setup exits' on finish page",
            ID_CHK_HIDE_LAUNCH,
            app.font_ui,
        );
        app.c.lbl_status = mk_label(app, parent, "Status", 0);
        app.c.ed_status = mk_edit(app, parent, ID_ED_STATUS, true);

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

    // ---- layout ------------------------------------------------------------------

    unsafe fn left_hwnds(app: &App) -> Vec<HWND> {
        let mut v: Vec<HWND> = app.c.labels.clone();
        v.extend([
            app.c.ed_name,
            app.c.ed_version,
            app.c.ed_publisher,
            app.c.ed_appid,
            app.c.ed_mainexe,
            app.c.ed_source,
            app.c.ed_output,
            app.c.ed_stub,
            app.c.btn_browse_stub,
            app.c.cb_scope,
            app.c.ed_trayarg,
            app.c.ed_startuparg,
            app.c.chk_tray,
            app.c.chk_hide_desktop,
            app.c.chk_hide_launch,
            app.c.lbl_status,
            app.c.ed_status,
        ]);
        v
    }

    unsafe fn layout(app: &mut App, hwnd: HWND) {
        let mut rc: RECT = std::mem::zeroed();
        let _ = GetClientRect(hwnd, &mut rc);
        let w = (rc.right - rc.left).max(0);
        let h = (rc.bottom - rc.top).max(0);
        let toolbar_h = 44;

        move_to(app.c.toolbar, 0, 0, w, toolbar_h);
        move_to(app.c.btn_load, 8, 8, 122, 28);
        move_to(app.c.btn_save, 136, 8, 122, 28);
        move_to(app.c.btn_validate, 264, 8, 92, 28);
        move_to(app.c.btn_build, 362, 8, 132, 28);
        move_to(app.c.btn_toggle_right, w - 118, 8, 110, 28);
        move_to(app.c.btn_toggle_left, w - 254, 8, 130, 28);

        let cy = toolbar_h;
        let ch = (h - toolbar_h).max(0);
        let gap = 4;
        let (lw, rw) = if app.left_collapsed && app.right_collapsed {
            (0, 0)
        } else if app.left_collapsed {
            (0, w)
        } else if app.right_collapsed {
            (w, 0)
        } else {
            let l = ((w - gap) * 46) / 100;
            (l, w - gap - l)
        };
        app.geom_left = Rect {
            x: 0,
            y: cy,
            w: lw,
            h: ch,
        };
        let rx = if lw == 0 {
            0
        } else if rw == 0 {
            w
        } else {
            lw + gap
        };
        app.geom_right = Rect {
            x: rx,
            y: cy,
            w: rw,
            h: ch,
        };

        show(app.c.panel_left, lw > 0);
        show(app.c.panel_right, rw > 0);
        if lw > 0 {
            move_to(app.c.panel_left, 0, cy, lw, ch);
        }
        if rw > 0 {
            move_to(app.c.panel_right, rx, cy, rw, ch);
        }
        layout_left(app);
        layout_right(app);
    }

    unsafe fn layout_left(app: &mut App) {
        let g = app.geom_left;
        if g.w <= 0 {
            for h in left_hwnds(app) {
                show(h, false);
            }
            return;
        }
        for h in left_hwnds(app) {
            show(h, true);
        }
        let label_x = g.x + 12;
        let edit_x = g.x + 142;
        let edit_w = (g.w - 154).max(80);
        let row = |i: i32| g.y + 12 + i * 30;
        // rows: 0 name, 1 version, 2 publisher, 3 app id, 4 main exe,
        // 5 source, 6 output, 7 stub, 8 scope, 9 tray arg, 10 startup arg
        let edits = [
            app.c.ed_name,
            app.c.ed_version,
            app.c.ed_publisher,
            app.c.ed_appid,
            app.c.ed_mainexe,
            app.c.ed_source,
            app.c.ed_output,
            app.c.ed_stub,
            app.c.cb_scope,
            app.c.ed_trayarg,
            app.c.ed_startuparg,
        ];
        for (i, label) in app.c.labels.clone().iter().enumerate() {
            move_to(*label, label_x, row(i as i32) + 3, 124, 20);
        }
        for (i, edit) in edits.iter().enumerate() {
            let hh = if *edit == app.c.cb_scope { 124 } else { 24 };
            move_to(*edit, edit_x, row(i as i32), edit_w, hh);
        }
        // Stub browse button sits at the end of the stub row (row 7).
        move_to(app.c.btn_browse_stub, edit_x + edit_w + 4, row(7), 34, 24);
        move_to(app.c.ed_stub, edit_x, row(7), (edit_w - 38).max(60), 24);

        let chk_x = g.x + 142;
        let chk_w = (g.w - 154).max(80);
        let chk_y = g.y + 12 + 11 * 30;
        move_to(app.c.chk_tray, chk_x, chk_y, chk_w, 22);
        move_to(app.c.chk_hide_desktop, chk_x, chk_y + 26, chk_w, 22);
        move_to(app.c.chk_hide_launch, chk_x, chk_y + 52, chk_w, 22);

        let status_label_y = chk_y + 84;
        move_to(app.c.lbl_status, label_x, status_label_y, 124, 20);
        let status_y = status_label_y + 22;
        let status_h = (g.y + g.h - status_y - 10).max(30);
        move_to(app.c.ed_status, g.x + 12, status_y, g.w - 24, status_h);
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
        app.cfg.app.name = get_text(app.c.ed_name);
        app.cfg.app.version = get_text(app.c.ed_version);
        app.cfg.app.publisher = get_text(app.c.ed_publisher);
        app.cfg.app.app_id = get_text(app.c.ed_appid);
        app.cfg.app.main_exe = get_text(app.c.ed_mainexe);
        app.cfg.install.source_dir = get_text(app.c.ed_source);
        app.output_dir = get_text(app.c.ed_output);
        app.stub_path = get_text(app.c.ed_stub);
        app.cfg.install.scope = match combo_sel(app.c.cb_scope) {
            1 => InstallScope::PerMachine,
            2 => InstallScope::DualMode,
            _ => InstallScope::PerUser,
        };
        app.cfg.tray.tray_arg = get_text(app.c.ed_trayarg);
        app.cfg.tray.startup_tray_arg = get_text(app.c.ed_startuparg);
        app.cfg.tray.supports_tray = get_check(app.c.chk_tray);
        app.cfg.finish.hide_desktop_shortcut = get_check(app.c.chk_hide_desktop);
        app.cfg.finish.hide_launch_after_setup = get_check(app.c.chk_hide_launch);
    }

    unsafe fn apply_config_to_ui(app: &mut App) {
        app.refreshing = true;
        set_text(app.c.ed_name, &app.cfg.app.name.clone());
        set_text(app.c.ed_version, &app.cfg.app.version.clone());
        set_text(app.c.ed_publisher, &app.cfg.app.publisher.clone());
        set_text(app.c.ed_appid, &app.cfg.app.app_id.clone());
        set_text(app.c.ed_mainexe, &app.cfg.app.main_exe.clone());
        set_text(app.c.ed_source, &app.cfg.install.source_dir.clone());
        set_text(app.c.ed_output, &app.output_dir.clone());
        set_text(app.c.ed_stub, &app.stub_path.clone());
        set_text(app.c.ed_trayarg, &app.cfg.tray.tray_arg.clone());
        set_text(app.c.ed_startuparg, &app.cfg.tray.startup_tray_arg.clone());
        combo_set_sel(
            app.c.cb_scope,
            match app.cfg.install.scope {
                InstallScope::PerUser => 0,
                InstallScope::PerMachine => 1,
                InstallScope::DualMode => 2,
            },
        );
        combo_set_sel(app.c.cb_page, app.page as i32);
        set_check(app.c.chk_tray, app.cfg.tray.supports_tray);
        set_check(app.c.chk_hide_desktop, app.cfg.finish.hide_desktop_shortcut);
        set_check(
            app.c.chk_hide_launch,
            app.cfg.finish.hide_launch_after_setup,
        );
        app.refreshing = false;
    }

    unsafe fn set_status(app: &App, text: &str) {
        set_text(app.c.ed_status, text);
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

    fn is_project_edit(id: i32) -> bool {
        matches!(
            id,
            ID_ED_NAME
                | ID_ED_VERSION
                | ID_ED_PUBLISHER
                | ID_ED_APPID
                | ID_ED_MAINEXE
                | ID_ED_SOURCE
                | ID_ED_OUTPUT
                | ID_ED_STUB
                | ID_ED_TRAYARG
                | ID_ED_STARTUPARG
        )
    }

    unsafe fn handle_command(app: &mut App, hwnd: HWND, wparam: WPARAM) {
        let id = (wparam & 0xFFFF) as i32;
        let code = ((wparam >> 16) & 0xFFFF) as u32;
        if code == EN_CHANGE {
            if !app.refreshing && is_project_edit(id) {
                sync_config_from_ui(app);
                refresh_preview(app);
            }
            return;
        }
        if code == CBN_SELCHANGE {
            if id == ID_CB_SCOPE {
                sync_config_from_ui(app);
                refresh_preview(app);
            } else if id == ID_CB_PAGE {
                let sel = combo_sel(app.c.cb_page);
                if sel >= 0 {
                    app.page = (sel as usize).min(PAGE_FINISH);
                }
                refresh_preview(app);
            }
            return;
        }
        if code != BN_CLICKED {
            return;
        }
        match id {
            ID_BTN_LOAD => do_load(app, hwnd),
            ID_BTN_SAVE => do_save(app, hwnd),
            ID_BTN_VALIDATE => do_validate(app),
            ID_BTN_BUILD => do_build(app),
            ID_BTN_BROWSE_STUB => {
                if let Some(path) = file_dialog(
                    hwnd,
                    false,
                    "Executable files (*.exe)\0*.exe\0All files (*.*)\0*.*\0",
                    "exe",
                    "",
                ) {
                    set_text(app.c.ed_stub, &path.display().to_string());
                    sync_config_from_ui(app);
                }
            }
            ID_BTN_TOGGLE_LEFT => {
                app.left_collapsed = !app.left_collapsed;
                set_text(
                    app.c.btn_toggle_left,
                    if app.left_collapsed {
                        "Show left panel"
                    } else {
                        "Hide left panel"
                    },
                );
                layout(app, hwnd);
            }
            ID_BTN_TOGGLE_RIGHT => {
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
            ID_CHK_TRAY | ID_CHK_HIDE_DESKTOP | ID_CHK_HIDE_LAUNCH => {
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
        std::fs::write(&setup_path, &exe)
            .map_err(|e| format!("could not write {}: {e}", setup_path.display()))?;
        let sums = sha256sums_txt(&[(setup_name.as_str(), exe.as_slice())]);
        let sums_path = out_dir.join("SHA256SUMS.txt");
        std::fs::write(&sums_path, sums.as_bytes())
            .map_err(|e| format!("could not write {}: {e}", sums_path.display()))?;
        Ok(format!(
            "Build complete.\n\nSetup: {}\nFiles packed: {}\nSetup size: {} bytes\nSHA-256: {}\n\nChecksums: {}",
            setup_path.display(),
            entries.len(),
            exe.len(),
            sha256_hex(&exe),
            sums_path.display()
        ))
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
