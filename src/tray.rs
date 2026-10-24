//! Resident tray mode. Windows-only: a notification-area icon
//! whose menu restores hidden programs, launches new ones hidden
//! (file picker), and restores everything on exit — while a
//! once-a-second timer keeps session windows hidden, so dialogs a
//! hidden program opens later are tucked away too.
//!
//! Pure Win32 FFI like the rest of HushRun — no shell crates. The
//! FFI struct layouts are pinned by compile-time size assertions
//! (64-bit) so a layout mistake fails the build, not the desktop.

#[cfg(not(windows))]
pub fn run() -> Result<String, String> {
    Err("the HushRun tray is only available on Windows".to_string())
}

/// Menu text for one entry in the "hide a running program" list:
/// the window title, with the executable name appended when it
/// adds information, capped so the menu stays a sane width.
/// Pure — tested on every platform.
// Used by the tray's Windows module; elsewhere only tests call it.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn menu_label(title: &str, exe: &str) -> String {
    let mut label = if exe.is_empty() || title.eq_ignore_ascii_case(exe) {
        title.to_string()
    } else {
        format!("{title} — {exe}")
    };
    if label.chars().count() > 56 {
        label = label.chars().take(55).collect::<String>() + "\u{2026}";
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_combine_title_and_exe() {
        assert_eq!(
            menu_label("Quarterly report - Editor", "editor.exe"),
            "Quarterly report - Editor — editor.exe"
        );
        assert_eq!(menu_label("editor.exe", "editor.exe"), "editor.exe");
        assert_eq!(menu_label("Just a title", ""), "Just a title");
    }

    #[test]
    fn labels_are_capped() {
        let long = "x".repeat(80);
        let label = menu_label(&long, "app.exe");
        assert_eq!(label.chars().count(), 56);
        assert!(label.ends_with('\u{2026}'));
    }
}

#[cfg(windows)]
pub fn run() -> Result<String, String> {
    windows::run()
}

#[cfg(windows)]
mod windows {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use crate::app;
    use crate::config::{self, Config, ProgramEntry};
    use crate::platform;
    use crate::session::{self, Session};

    type Hwnd = *mut c_void;
    type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

    const WM_CLOSE: u32 = 0x0010;
    const WM_DESTROY: u32 = 0x0002;
    const WM_TIMER: u32 = 0x0113;
    const WM_NULL: u32 = 0x0000;
    const WM_HOTKEY: u32 = 0x0312;
    const TRAY_MSG: u32 = 0x8000 + 1; // WM_APP + 1
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;

    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIF_MESSAGE: u32 = 0x1;
    const NIF_ICON: u32 = 0x2;
    const NIF_TIP: u32 = 0x4;
    const NIF_INFO: u32 = 0x10;
    const NIIF_INFO: u32 = 0x1;
    const NIIF_WARNING: u32 = 0x2;

    const MF_STRING: u32 = 0x0000;
    const MF_GRAYED: u32 = 0x0001;
    const MF_DISABLED: u32 = 0x0002;
    const MF_SEPARATOR: u32 = 0x0800;
    const MF_POPUP: u32 = 0x0010;
    const TPM_RETURNCMD: u32 = 0x0100;
    const TPM_RIGHTBUTTON: u32 = 0x0002;

    const ID_RESTORE_BASE: u32 = 1000;
    const ID_RUN: u32 = 2000;
    const ID_RESTORE_ALL: u32 = 2001;
    const ID_EXIT: u32 = 2002;
    const ID_SETTINGS: u32 = 2003;
    const ID_EXPORT: u32 = 2004;
    const ID_IMPORT: u32 = 2005;
    const ID_ADMIN: u32 = 2006;
    const ID_ABOUT: u32 = 2007;
    const ID_HIDE_BASE: u32 = 3000;
    const ID_WIN_BASE: u32 = 4000;
    const MF_CHECKED: u32 = 0x0008;

    const MOD_ALT: u32 = 0x0001;
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_NOREPEAT: u32 = 0x4000;
    const HOTKEY_HIDE: i32 = 1;
    const HOTKEY_RESTORE: i32 = 2;
    const VK_H: u32 = 0x48;
    const VK_U: u32 = 0x55;

    const OFN_PATHMUSTEXIST: u32 = 0x0800;
    const OFN_FILEMUSTEXIST: u32 = 0x1000;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const MB_OK: u32 = 0x0;
    const MB_YESNO: u32 = 0x4;
    const MB_ICONINFORMATION: u32 = 0x40;
    const MB_ICONERROR: u32 = 0x10;
    const MB_ICONQUESTION: u32 = 0x20;
    const IDYES: i32 = 6;

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: Point,
    }

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        lpfn_wnd_proc: WndProc,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: *mut c_void,
        h_icon: *mut c_void,
        h_cursor: *mut c_void,
        hbr_background: *mut c_void,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: *mut c_void,
    }

    #[repr(C)]
    struct NotifyIconDataW {
        cb_size: u32,
        h_wnd: Hwnd,
        u_id: u32,
        u_flags: u32,
        u_callback_message: u32,
        h_icon: *mut c_void,
        sz_tip: [u16; 128],
        dw_state: u32,
        dw_state_mask: u32,
        sz_info: [u16; 256],
        u_timeout: u32,
        sz_info_title: [u16; 64],
        dw_info_flags: u32,
        guid_item: [u8; 16],
        h_balloon_icon: *mut c_void,
    }

    #[repr(C)]
    struct OpenFileNameW {
        l_struct_size: u32,
        hwnd_owner: Hwnd,
        h_instance: *mut c_void,
        lpstr_filter: *const u16,
        lpstr_custom_filter: *mut u16,
        n_max_cust_filter: u32,
        n_filter_index: u32,
        lpstr_file: *mut u16,
        n_max_file: u32,
        n_max_file_title: u32,
        lpstr_file_title: *mut u16,
        lpstr_initial_dir: *const u16,
        lpstr_title: *const u16,
        flags: u32,
        n_file_offset: u16,
        n_file_extension: u16,
        lpstr_def_ext: *const u16,
        l_cust_data: isize,
        lpfn_hook: *mut c_void,
        lp_template_name: *const u16,
        pv_reserved: *mut c_void,
        dw_reserved: u32,
        flags_ex: u32,
    }

    #[repr(C)]
    struct IconInfo {
        f_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        hbm_mask: *mut c_void,
        hbm_color: *mut c_void,
    }

    // The layouts above are the documented 64-bit SDK layouts; if
    // one is wrong, this fails at compile time on the Windows check.
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(std::mem::size_of::<NotifyIconDataW>() == 976);
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(std::mem::size_of::<OpenFileNameW>() == 144);
    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(std::mem::size_of::<WndClassExW>() == 80);

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(wc: *const WndClassExW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Hwnd,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn PostQuitMessage(code: i32);
        fn PostMessageW(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> i32;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn SetTimer(hwnd: Hwnd, id: usize, ms: u32, proc: *mut c_void) -> usize;
        fn CreatePopupMenu() -> *mut c_void;
        fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
        fn DestroyMenu(menu: *mut c_void) -> i32;
        fn TrackPopupMenu(
            menu: *mut c_void,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            hwnd: Hwnd,
            rect: *const c_void,
        ) -> u32;
        fn GetCursorPos(pt: *mut Point) -> i32;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
        fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
        fn DestroyIcon(icon: *mut c_void) -> i32;
        fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, kind: u32) -> i32;
        fn RegisterHotKey(hwnd: Hwnd, id: i32, modifiers: u32, vk: u32) -> i32;
        fn UnregisterHotKey(hwnd: Hwnd, id: i32) -> i32;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn SendMessageW(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
        fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
        fn IsWindow(hwnd: Hwnd) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetStockObject(kind: i32) -> *mut c_void;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *const NotifyIconDataW) -> i32;
        fn ShellExecuteW(
            hwnd: Hwnd,
            operation: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> *mut c_void;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateBitmap(w: i32, h: i32, planes: u32, bits: u32, data: *const c_void)
            -> *mut c_void;
        fn DeleteObject(obj: *mut c_void) -> i32;
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetOpenFileNameW(ofn: *mut OpenFileNameW) -> i32;
        fn GetSaveFileNameW(ofn: *mut OpenFileNameW) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn CreateMutexW(attrs: *mut c_void, initial_owner: i32, name: *const u16) -> *mut c_void;
        fn GetLastError() -> u32;
        fn FreeConsole() -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }

    struct TrayState {
        hwnd: Hwnd,
        icon: *mut c_void,
        icon_on: *mut c_void,
        icon_off: *mut c_void,
        mutex: *mut c_void,
        menu_sessions: Vec<Session>,
        menu_hide: Vec<(u32, String)>,
        menu_windows: Vec<Vec<isize>>,
        settings_hwnd: Hwnd,
        tip_count: usize,
        taskbar_msg: u32,
        first_tick_done: bool,
    }

    thread_local! {
        static STATE: RefCell<TrayState> = const { RefCell::new(TrayState {
            hwnd: std::ptr::null_mut(),
            icon: std::ptr::null_mut(),
            icon_on: std::ptr::null_mut(),
            icon_off: std::ptr::null_mut(),
            mutex: std::ptr::null_mut(),
            menu_sessions: Vec::new(),
            menu_hide: Vec::new(),
            menu_windows: Vec::new(),
            settings_hwnd: std::ptr::null_mut(),
            tip_count: usize::MAX,
            taskbar_msg: 0,
            first_tick_done: false,
        }) };
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn display_name(program: &str) -> String {
        program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(program)
            .to_string()
    }

    fn tip_text(count: usize) -> String {
        match count {
            0 => "HushRun — nothing hidden".to_string(),
            1 => "HushRun — 1 program hidden".to_string(),
            n => format!("HushRun — {n} programs hidden"),
        }
    }

    fn nid_base(hwnd: Hwnd, icon: *mut c_void) -> NotifyIconDataW {
        let mut nid: NotifyIconDataW = unsafe { std::mem::zeroed() };
        nid.cb_size = std::mem::size_of::<NotifyIconDataW>() as u32;
        nid.h_wnd = hwnd;
        nid.u_id = 1;
        nid.h_icon = icon;
        nid
    }

    /// A quiet balloon/toast from the tray icon. Used for the
    /// events the user caused or would want to know about: a
    /// program hidden, restored, or exiting while hidden.
    fn balloon(hwnd: Hwnd, icon: *mut c_void, text: &str, warning: bool) {
        let mut nid = nid_base(hwnd, icon);
        nid.u_flags = NIF_INFO;
        let title = wide("HushRun");
        let body = wide(text);
        let tlen = title.len().min(63);
        nid.sz_info_title[..tlen].copy_from_slice(&title[..tlen]);
        let blen = body.len().min(255);
        nid.sz_info[..blen].copy_from_slice(&body[..blen]);
        nid.dw_info_flags = if warning { NIIF_WARNING } else { NIIF_INFO };
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    fn balloon_from_state(hwnd: Hwnd, text: &str, warning: bool) {
        if !config::load(&config::config_path()).notifications {
            return;
        }
        let icon = STATE.with(|st| st.borrow().icon);
        if !icon.is_null() {
            balloon(hwnd, icon, text, warning);
        }
    }

    /// Keep the icon and tooltip in step with how much is hidden:
    /// bright icon while anything is tucked away, dim when not.
    fn refresh_indicator(hwnd: Hwnd, count: usize) {
        let (changed, icon) = STATE.with(|st| {
            let mut st = st.borrow_mut();
            let want = if count > 0 { st.icon_on } else { st.icon_off };
            let changed = st.tip_count != count || st.icon != want;
            if changed {
                st.tip_count = count;
                st.icon = want;
            }
            (changed, want)
        });
        if changed && !icon.is_null() {
            let mut nid = nid_base(hwnd, icon);
            nid.u_flags = NIF_ICON | NIF_TIP;
            let text = wide(&tip_text(count));
            let len = text.len().min(127);
            nid.sz_tip[..len].copy_from_slice(&text[..len]);
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        }
    }

    /// Load the store, drop sessions whose process exited, and
    /// return the live ones. Saves when anything was pruned.
    fn live_sessions() -> Vec<Session> {
        let path = session::store_path();
        let mut sessions = session::load(&path);
        let before = sessions.len();
        sessions.retain(|s| platform::process_alive(s.pid));
        if sessions.len() != before {
            let _ = session::save(&path, &sessions);
        }
        sessions
    }

    fn tick(hwnd: Hwnd) {
        let path = session::store_path();
        let mut sessions = session::load(&path);
        let mut changed = false;
        let mut live = Vec::new();
        let mut died: Vec<String> = Vec::new();
        for mut s in sessions.drain(..) {
            if !platform::process_alive(s.pid) {
                changed = true;
                died.push(display_name(&s.program));
                continue;
            }
            for w in platform::windows_of(s.pid) {
                if platform::is_window_visible(w) && platform::set_window_visible(w, false) {
                    s.windows_hidden += 1;
                    changed = true;
                }
            }
            live.push(s);
        }
        if changed {
            let _ = session::save(&path, &live);
        }
        let count = live.len();
        let first = STATE.with(|st| {
            let mut st = st.borrow_mut();
            let first = !st.first_tick_done;
            st.first_tick_done = true;
            first
        });
        refresh_indicator(hwnd, count);
        // Programs that exited while hidden are worth one balloon —
        // but not for sessions that were already dead when the tray
        // started (the first tick just cleans those up).
        if !first && !died.is_empty() {
            let text = if died.len() == 1 {
                format!("{} exited while hidden", died[0])
            } else {
                format!("{} programs exited while hidden", died.len())
            };
            balloon_from_state(hwnd, &text, false);
        }
    }

    fn upsert_session(pid: u32, program: &str, windows_hidden: u32) {
        let path = session::store_path();
        let mut sessions = session::load(&path);
        if let Some(s) = sessions.iter_mut().find(|s| s.pid == pid) {
            s.windows_hidden += windows_hidden;
        } else {
            sessions.push(Session {
                pid,
                program: program.to_string(),
                args: Vec::new(),
                started_unix: app::now_unix(),
                windows_hidden,
            });
        }
        let _ = session::save(&path, &sessions);
    }

    /// Hide every visible window of a process and record it — the
    /// shared action behind the menu's "hide a running program"
    /// list and the Ctrl+Alt+H hotkey.
    fn hide_process(hwnd: Hwnd, pid: u32, program: &str) {
        let mut hidden = 0usize;
        for w in platform::windows_of(pid) {
            if platform::is_window_visible(w) && platform::set_window_visible(w, false) {
                hidden += 1;
            }
        }
        let name = display_name(program);
        if hidden > 0 {
            upsert_session(pid, program, hidden as u32);
            balloon_from_state(hwnd, &format!("{name} is now hidden"), false);
        } else {
            balloon_from_state(hwnd, &format!("No visible window found for {name}"), true);
        }
        tick(hwnd);
    }

    fn hide_foreground(hwnd: Hwnd) {
        let Some(hwnd_fg) = platform::foreground_window() else {
            return;
        };
        let Some(pid) = platform::window_pid(hwnd_fg) else {
            return;
        };
        if pid == platform::own_pid() {
            return;
        }
        let program = platform::exe_name(pid).unwrap_or_else(|| format!("pid {pid}"));
        hide_process(hwnd, pid, &program);
    }

    fn restore_session(hwnd: Hwnd, s: &Session) {
        app::show_all(s.pid);
        let path = session::store_path();
        let mut sessions = session::load(&path);
        sessions.retain(|x| x.pid != s.pid);
        let _ = session::save(&path, &sessions);
        balloon_from_state(
            hwnd,
            &format!("{} restored", display_name(&s.program)),
            false,
        );
        tick(hwnd);
    }

    fn restore_everything() -> usize {
        let sessions = live_sessions();
        let count = sessions.len();
        for s in &sessions {
            app::show_all(s.pid);
        }
        let _ = session::save(&session::store_path(), &[]);
        count
    }

    fn show_menu(hwnd: Hwnd) {
        let sessions = live_sessions();
        let own = platform::own_pid();
        let hide_targets: Vec<(u32, String, String)> = platform::visible_apps()
            .into_iter()
            .filter(|(pid, _)| *pid != own && !sessions.iter().any(|s| s.pid == *pid))
            .map(|(pid, title)| {
                let exe = platform::exe_name(pid).unwrap_or_default();
                (pid, title, exe)
            })
            .collect();
        STATE.with(|st| {
            let mut st = st.borrow_mut();
            st.menu_sessions = sessions.clone();
            st.menu_windows = sessions
                .iter()
                .map(|s| platform::windows_of(s.pid))
                .collect();
            st.menu_hide = hide_targets
                .iter()
                .map(|(pid, title, exe)| {
                    (
                        *pid,
                        if exe.is_empty() {
                            title.clone()
                        } else {
                            exe.clone()
                        },
                    )
                })
                .collect();
        });
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        let append = |flags: u32, id: usize, text: &str| {
            let w = wide(text);
            unsafe { AppendMenuW(menu, flags, id, w.as_ptr()) };
        };
        append(
            MF_STRING | MF_GRAYED | MF_DISABLED,
            0,
            &tip_text(sessions.len()),
        );
        append(MF_SEPARATOR, 0, "");
        // One submenu per hidden program: restore all of its
        // windows, or just one (hidden windows are checkmarked).
        let menu_windows = STATE.with(|st| st.borrow().menu_windows.clone());
        for (i, s) in sessions.iter().enumerate() {
            let sub = unsafe { CreatePopupMenu() };
            if sub.is_null() {
                continue;
            }
            let all = wide("All windows");
            unsafe { AppendMenuW(sub, MF_STRING, (ID_RESTORE_BASE as usize) + i, all.as_ptr()) };
            let wins = menu_windows.get(i).cloned().unwrap_or_default();
            if !wins.is_empty() {
                let sep = wide("");
                unsafe { AppendMenuW(sub, MF_SEPARATOR, 0, sep.as_ptr()) };
            }
            for (j, w) in wins.iter().enumerate().take(31) {
                let title = platform::window_title(*w);
                let label = if title.is_empty() {
                    "(untitled window)".to_string()
                } else {
                    crate::tray::menu_label(&title, "")
                };
                let flags = if platform::is_window_visible(*w) {
                    MF_STRING
                } else {
                    MF_STRING | MF_CHECKED
                };
                let lw = wide(&label);
                unsafe {
                    AppendMenuW(
                        sub,
                        flags,
                        (ID_WIN_BASE as usize) + i * 32 + j + 1,
                        lw.as_ptr(),
                    )
                };
            }
            let label = wide(&format!(
                "Restore {} (pid {})",
                display_name(&s.program),
                s.pid
            ));
            unsafe { AppendMenuW(menu, MF_POPUP, sub as usize, label.as_ptr()) };
        }
        if !sessions.is_empty() {
            append(MF_STRING, ID_RESTORE_ALL as usize, "Restore all");
            append(MF_SEPARATOR, 0, "");
        }
        // "Hide a running program" submenu.
        let sub = unsafe { CreatePopupMenu() };
        if !sub.is_null() {
            if hide_targets.is_empty() {
                let w = wide("(no visible programs)");
                unsafe { AppendMenuW(sub, MF_STRING | MF_GRAYED | MF_DISABLED, 0, w.as_ptr()) };
            } else {
                for (i, (_, title, exe)) in hide_targets.iter().enumerate() {
                    let w = wide(&crate::tray::menu_label(title, exe));
                    unsafe { AppendMenuW(sub, MF_STRING, (ID_HIDE_BASE as usize) + i, w.as_ptr()) };
                }
            }
            let w = wide("Hide a running program");
            unsafe { AppendMenuW(menu, MF_POPUP, sub as usize, w.as_ptr()) };
        }
        append(MF_STRING, ID_RUN as usize, "Run a program hidden\u{2026}");
        append(MF_SEPARATOR, 0, "");
        append(MF_STRING, ID_SETTINGS as usize, "Settings\u{2026}");
        append(MF_STRING, ID_EXPORT as usize, "Export settings\u{2026}");
        append(MF_STRING, ID_IMPORT as usize, "Import settings\u{2026}");
        append(MF_STRING, ID_ADMIN as usize, "Restart as administrator");
        append(MF_SEPARATOR, 0, "");
        append(MF_STRING, ID_ABOUT as usize, "About HushRun");
        append(MF_STRING, ID_EXIT as usize, "Exit (restore all)");
        append(MF_SEPARATOR, 0, "");
        append(
            MF_STRING | MF_GRAYED | MF_DISABLED,
            0,
            "Hotkeys: Ctrl+Alt+H hide window \u{b7} Ctrl+Alt+U restore all",
        );

        let mut pt = Point { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
        }
        let cmd = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                0,
                hwnd,
                std::ptr::null(),
            )
        };
        unsafe {
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
        }
        match cmd {
            ID_EXIT => {
                restore_everything();
                unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
            }
            ID_RESTORE_ALL => {
                if restore_everything() > 0 {
                    balloon_from_state(hwnd, "All programs restored", false);
                }
                tick(hwnd);
            }
            ID_RUN => pick_and_run(hwnd),
            ID_SETTINGS => open_settings(hwnd),
            ID_EXPORT => export_via_dialog(hwnd),
            ID_IMPORT => import_via_dialog(hwnd),
            ID_ADMIN => restart_as_admin(hwnd),
            ID_ABOUT => about_box(hwnd),
            id if id >= ID_WIN_BASE => {
                let rel = (id - ID_WIN_BASE) as usize;
                let (s_idx, w_idx) = (rel / 32, rel % 32);
                if w_idx > 0 {
                    restore_one_window(hwnd, s_idx, w_idx - 1);
                }
            }
            id if id >= ID_HIDE_BASE => {
                let idx = (id - ID_HIDE_BASE) as usize;
                let target = STATE.with(|st| st.borrow().menu_hide.get(idx).cloned());
                if let Some((pid, program)) = target {
                    hide_process(hwnd, pid, &program);
                }
            }
            id if id >= ID_RESTORE_BASE => {
                let idx = (id - ID_RESTORE_BASE) as usize;
                let target = STATE.with(|st| st.borrow().menu_sessions.get(idx).cloned());
                if let Some(s) = target {
                    restore_session(hwnd, &s);
                }
            }
            _ => {}
        }
    }

    /// The common-dialog file picker (open or save). Returns the
    /// chosen path, or None when the user cancelled.
    fn file_dialog(
        hwnd: Hwnd,
        save: bool,
        filter_parts: &[&str],
        initial_name: &str,
    ) -> Option<String> {
        let mut filter: Vec<u16> = Vec::new();
        for part in filter_parts {
            filter.extend(part.encode_utf16());
            filter.push(0);
        }
        filter.push(0);
        let mut buffer = vec![0u16; 520];
        for (i, u) in initial_name.encode_utf16().enumerate() {
            if i < buffer.len() - 1 {
                buffer[i] = u;
            }
        }
        let mut ofn: OpenFileNameW = unsafe { std::mem::zeroed() };
        ofn.l_struct_size = std::mem::size_of::<OpenFileNameW>() as u32;
        ofn.hwnd_owner = hwnd;
        ofn.lpstr_filter = filter.as_ptr();
        ofn.lpstr_file = buffer.as_mut_ptr();
        ofn.n_max_file = buffer.len() as u32;
        ofn.flags = OFN_PATHMUSTEXIST | if save { 0 } else { OFN_FILEMUSTEXIST };
        let ok = unsafe {
            if save {
                GetSaveFileNameW(&mut ofn)
            } else {
                GetOpenFileNameW(&mut ofn)
            }
        };
        if ok == 0 {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buffer[..len]))
    }

    fn info_box(hwnd: Hwnd, text: &str, error: bool) {
        let w = wide(text);
        let caption = wide("HushRun");
        let kind = if error {
            MB_OK | MB_ICONERROR
        } else {
            MB_OK | MB_ICONINFORMATION
        };
        unsafe { MessageBoxW(hwnd, w.as_ptr(), caption.as_ptr(), kind) };
    }

    /// Show exactly one window of a hidden program; when none of
    /// its windows remain hidden, the session is complete and is
    /// dropped from the store.
    fn restore_one_window(hwnd: Hwnd, s_idx: usize, w_idx: usize) {
        let target = STATE.with(|st| {
            let st = st.borrow();
            let session = st.menu_sessions.get(s_idx)?;
            let hwnd_win = st.menu_windows.get(s_idx)?.get(w_idx)?;
            Some((session.clone(), *hwnd_win))
        });
        let Some((session, hwnd_win)) = target else {
            return;
        };
        platform::set_window_visible(hwnd_win, true);
        let still_hidden = platform::windows_of(session.pid)
            .iter()
            .any(|w| !platform::is_window_visible(*w));
        if !still_hidden {
            let path = session::store_path();
            let mut sessions = session::load(&path);
            sessions.retain(|x| x.pid != session.pid);
            let _ = session::save(&path, &sessions);
        }
        balloon_from_state(
            hwnd,
            &format!("One {} window restored", display_name(&session.program)),
            false,
        );
        tick(hwnd);
    }

    fn export_via_dialog(hwnd: Hwnd) {
        let Some(dest) = file_dialog(
            hwnd,
            true,
            &["JSON settings (*.json)", "*.json", "All files (*.*)", "*.*"],
            "hushrun-settings.json",
        ) else {
            return;
        };
        match app::export_settings(std::path::Path::new(&dest)) {
            Ok(message) => balloon_from_state(hwnd, &message, false),
            Err(message) => info_box(hwnd, &message, true),
        }
    }

    fn import_via_dialog(hwnd: Hwnd) {
        let Some(source) = file_dialog(
            hwnd,
            false,
            &["JSON settings (*.json)", "*.json", "All files (*.*)", "*.*"],
            "",
        ) else {
            return;
        };
        match app::import_settings(std::path::Path::new(&source)) {
            Ok(_) => balloon_from_state(hwnd, "Settings imported", false),
            Err(message) => info_box(hwnd, &message, true),
        }
    }

    /// Relaunch the tray elevated (so it can hide elevated
    /// programs). The single-instance mutex is released first so
    /// the new instance doesn't bounce off this one; if the UAC
    /// prompt is declined, this instance carries on.
    fn restart_as_admin(hwnd: Hwnd) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let old_mutex = STATE.with(|st| {
            let mut st = st.borrow_mut();
            std::mem::replace(&mut st.mutex, std::ptr::null_mut())
        });
        if !old_mutex.is_null() {
            unsafe { CloseHandle(old_mutex) };
        }
        let operation = wide("runas");
        let file = wide(&exe.to_string_lossy());
        let params = wide("tray");
        let result = unsafe {
            ShellExecuteW(
                hwnd,
                operation.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                1, // SW_SHOWNORMAL
            )
        };
        if (result as isize) > 32 {
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        } else {
            // Declined or failed: restore single-instance cover.
            let name = wide("HushRunTrayMutex");
            let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr()) };
            STATE.with(|st| st.borrow_mut().mutex = mutex);
            balloon_from_state(hwnd, "Administrator restart was cancelled", true);
        }
    }

    fn about_box(hwnd: Hwnd) {
        info_box(
            hwnd,
            &format!(
                "HushRun {}\nRun programs silently in the background.\n\nMIT License — Copyright (c) 2026 Sean Shaffer\nhttps://github.com/SeanShaffer88/HushRun",
                env!("CARGO_PKG_VERSION")
            ),
            false,
        );
    }

    fn pick_and_run(hwnd: Hwnd) {
        let Some(path) = file_dialog(
            hwnd,
            false,
            &["Programs (*.exe)", "*.exe", "All files (*.*)", "*.*"],
            "",
        ) else {
            return;
        };
        let pid = match platform::launch_hidden(&path, &[]) {
            Ok(pid) => pid,
            Err(message) => {
                let text = wide(&message);
                let caption = wide("HushRun");
                unsafe { MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR) };
                return;
            }
        };
        let name = display_name(&path);
        upsert_session(pid, &path, 0);
        balloon_from_state(hwnd, &format!("Started {name} hidden"), false);
        let watch = config::load(&config::config_path()).watch_seconds;
        std::thread::spawn(move || {
            let hidden = app::watch_hide(pid, watch);
            let store = session::store_path();
            let mut sessions = session::load(&store);
            if let Some(s) = sessions.iter_mut().find(|s| s.pid == pid) {
                s.windows_hidden = hidden as u32;
                let _ = session::save(&store, &sessions);
            }
        });
        tick(hwnd);
    }

    /// Sessions whose program exited while HushRun was away are
    /// relaunched hidden when the program file is still there —
    /// a reboot shouldn't lose what was tucked away.
    fn relaunch_dead_sessions() {
        let path = session::store_path();
        let cfg = config::load(&config::config_path());
        let mut sessions = session::load(&path);
        let mut relaunched = 0usize;
        let mut kept = Vec::new();
        for s in sessions.drain(..) {
            if platform::process_alive(s.pid) {
                kept.push(s);
                continue;
            }
            let launchable = (s.program.contains('\\') || s.program.contains('/'))
                && std::path::Path::new(&s.program).is_file();
            if launchable {
                if let Ok(pid) = platform::launch_hidden(&s.program, &s.args) {
                    kept.push(Session {
                        pid,
                        program: s.program.clone(),
                        args: s.args.clone(),
                        started_unix: app::now_unix(),
                        windows_hidden: 0,
                    });
                    relaunched += 1;
                    let watch = cfg.watch_seconds;
                    std::thread::spawn(move || {
                        let hidden = app::watch_hide(pid, watch);
                        let store = session::store_path();
                        let mut ss = session::load(&store);
                        if let Some(x) = ss.iter_mut().find(|x| x.pid == pid) {
                            x.windows_hidden = hidden as u32;
                            let _ = session::save(&store, &ss);
                        }
                    });
                }
            }
        }
        let _ = session::save(&path, &kept);
        if relaunched > 0 {
            let hwnd = STATE.with(|st| st.borrow().hwnd);
            balloon_from_state(
                hwnd,
                &format!(
                    "Relaunched {relaunched} program(s) hidden — they were tucked away before the last shutdown"
                ),
                false,
            );
        }
    }

    /// Auto-start entries: when the tray starts, hide these
    /// programs — launching them if needed — after their delay.
    fn run_auto_entries() {
        let cfg = config::load(&config::config_path());
        let sessions = session::load(&session::store_path());
        for entry in cfg.entries.iter().filter(|e| e.auto_start) {
            if sessions
                .iter()
                .any(|s| s.program == entry.program && platform::process_alive(s.pid))
            {
                continue;
            }
            let entry: ProgramEntry = entry.clone();
            let default_watch = cfg.watch_seconds;
            std::thread::spawn(move || {
                if entry.delay_seconds > 0 {
                    std::thread::sleep(std::time::Duration::from_secs(entry.delay_seconds));
                }
                let watch = entry.watch_seconds.unwrap_or(default_watch);
                let found = platform::find_process_by_name(entry.file_name());
                let pid = match found {
                    Some((pid, _)) => Some(pid),
                    None => {
                        if std::path::Path::new(&entry.program).is_file() {
                            platform::launch_hidden(&entry.program, &entry.args).ok()
                        } else {
                            None
                        }
                    }
                };
                if let Some(pid) = pid {
                    upsert_session(pid, &entry.program, 0);
                    let hidden = app::watch_hide(pid, watch);
                    let store = session::store_path();
                    let mut ss = session::load(&store);
                    if let Some(x) = ss.iter_mut().find(|x| x.pid == pid) {
                        x.windows_hidden = hidden as u32;
                        if x.args.is_empty() {
                            x.args = entry.args.clone();
                        }
                        let _ = session::save(&store, &ss);
                    }
                }
            });
        }
    }

    /// The tray icon: a near-black rounded square with a red border
    /// and a white pixel "H" — drawn into a bitmap, no resources.
    /// The dim variant (grey border and "H") signals "nothing
    /// hidden right now".
    fn create_app_icon(dim: bool) -> *mut c_void {
        const S: i32 = 32;
        const GLYPH: [u8; 7] = [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ];
        let mut px = vec![0u32; (S * S) as usize];
        for y in 0..S {
            for x in 0..S {
                let r = 7;
                let cx = x.clamp(r, S - 1 - r);
                let cy = y.clamp(r, S - 1 - r);
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy > r * r {
                    continue; // transparent corner
                }
                let edge = x < 2 || y < 2 || x >= S - 2 || y >= S - 2;
                let (border, glyph) = if dim {
                    (0xFF3A_3A3A, 0xFF6E_6E6E)
                } else {
                    (0xFFE5_484D, 0xFFF5_F5F5)
                };
                let mut col: u32 = if edge { border } else { 0xFF14_1414 };
                if (8..23).contains(&x) && (5..26).contains(&y) {
                    let (gx, gy) = ((x - 8) / 3, (y - 5) / 3);
                    if (GLYPH[gy as usize] >> (4 - gx)) & 1 == 1 {
                        col = glyph;
                    }
                }
                px[(y * S + x) as usize] = col;
            }
        }
        unsafe {
            let color = CreateBitmap(S, S, 1, 32, px.as_ptr() as *const c_void);
            let mask_bits = [0u8; 128]; // 32x32 monochrome, all zero
            let mask = CreateBitmap(S, S, 1, 1, mask_bits.as_ptr() as *const c_void);
            if color.is_null() || mask.is_null() {
                if !color.is_null() {
                    DeleteObject(color);
                }
                if !mask.is_null() {
                    DeleteObject(mask);
                }
                return LoadIconW(std::ptr::null_mut(), 32512 as *const u16);
            }
            let info = IconInfo {
                f_icon: 1,
                x_hotspot: 0,
                y_hotspot: 0,
                hbm_mask: mask,
                hbm_color: color,
            };
            let icon = CreateIconIndirect(&info);
            DeleteObject(color);
            DeleteObject(mask);
            if icon.is_null() {
                LoadIconW(std::ptr::null_mut(), 32512 as *const u16)
            } else {
                icon
            }
        }
    }

    /// Re-add the icon after Explorer restarts (its message is
    /// registered at startup, so the value is dynamic).
    fn re_add_icon(hwnd: Hwnd) {
        let (icon, count) = STATE.with(|st| {
            let st = st.borrow();
            (st.icon, st.tip_count)
        });
        if icon.is_null() {
            return;
        }
        let count = if count == usize::MAX {
            live_sessions().len()
        } else {
            count
        };
        let mut nid = nid_base(hwnd, icon);
        nid.u_flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.u_callback_message = TRAY_MSG;
        let tip = wide(&tip_text(count));
        nid.sz_tip[..tip.len()].copy_from_slice(&tip);
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    }

    unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize {
        let taskbar_msg = STATE.with(|st| st.borrow().taskbar_msg);
        if taskbar_msg != 0 && msg == taskbar_msg {
            re_add_icon(hwnd);
            return 0;
        }
        match msg {
            TRAY_MSG => {
                if matches!(l as u32, WM_RBUTTONUP | WM_LBUTTONUP | WM_LBUTTONDBLCLK) {
                    show_menu(hwnd);
                }
                0
            }
            WM_HOTKEY => {
                match w as i32 {
                    HOTKEY_HIDE => hide_foreground(hwnd),
                    HOTKEY_RESTORE => {
                        if restore_everything() > 0 {
                            balloon_from_state(hwnd, "All programs restored", false);
                        }
                        tick(hwnd);
                    }
                    _ => {}
                }
                0
            }
            WM_TIMER => {
                tick(hwnd);
                0
            }
            WM_CLOSE => {
                unsafe { DestroyWindow(hwnd) };
                0
            }
            WM_DESTROY => {
                let mut nid = nid_base(hwnd, std::ptr::null_mut());
                nid.u_flags = 0;
                unsafe {
                    UnregisterHotKey(hwnd, HOTKEY_HIDE);
                    UnregisterHotKey(hwnd, HOTKEY_RESTORE);
                    Shell_NotifyIconW(NIM_DELETE, &nid);
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
        }
    }

    // ---------- Settings window ----------
    //
    // A plain Win32 window with child controls, editing a working
    // copy of the Config: global toggles up top, the program
    // entries (profiles + auto-start rules) below. Field edits
    // commit into the working copy whenever the selection changes
    // or OK is pressed; OK writes the file and applies the
    // start-with-Windows preference.

    const WM_COMMAND_MSG: u32 = 0x0111;
    const WM_SETFONT_MSG: u32 = 0x0030;
    const BM_GETCHECK: u32 = 0x00F0;
    const BM_SETCHECK: u32 = 0x00F1;
    const LB_ADDSTRING: u32 = 0x0180;
    const LB_RESETCONTENT: u32 = 0x0184;
    const LB_DELETESTRING: u32 = 0x0182;
    const LB_GETCURSEL: u32 = 0x0188;
    const LB_SETCURSEL: u32 = 0x0186;
    const LBN_SELCHANGE: usize = 1;

    const SID_START: usize = 101;
    const SID_NOTIF: usize = 102;
    const SID_GWATCH: usize = 103;
    const SID_LIST: usize = 104;
    const SID_ADD: usize = 105;
    const SID_REMOVE: usize = 106;
    const SID_ARGS: usize = 107;
    const SID_WATCH: usize = 108;
    const SID_DELAY: usize = 109;
    const SID_AUTO: usize = 110;
    const SID_OK: usize = 112;
    const SID_CANCEL: usize = 113;

    struct SettingsState {
        config: Config,
        selected: Option<usize>,
        list: Hwnd,
        args: Hwnd,
        watch: Hwnd,
        delay: Hwnd,
        auto: Hwnd,
        start: Hwnd,
        notif: Hwnd,
        gwatch: Hwnd,
    }

    thread_local! {
        static SETTINGS: RefCell<Option<SettingsState>> = const { RefCell::new(None) };
    }

    fn get_check(c: Hwnd) -> bool {
        let state = unsafe { SendMessageW(c, BM_GETCHECK, 0, 0) };
        state == 1
    }

    fn set_check(c: Hwnd, on: bool) {
        unsafe { SendMessageW(c, BM_SETCHECK, if on { 1 } else { 0 }, 0) };
    }

    fn edit_text(c: Hwnd) -> String {
        let mut buf = [0u16; 1024];
        let len = unsafe { GetWindowTextW(c, buf.as_mut_ptr(), buf.len() as i32) };
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }

    fn set_edit(c: Hwnd, text: &str) {
        let w = wide(text);
        unsafe { SetWindowTextW(c, w.as_ptr()) };
    }

    /// Write the field editors into the selected entry.
    fn st_commit(st: &mut SettingsState) {
        let Some(idx) = st.selected else { return };
        let Some(entry) = st.config.entries.get_mut(idx) else {
            return;
        };
        entry.args = edit_text(st.args)
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        entry.watch_seconds = edit_text(st.watch)
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0);
        entry.delay_seconds = edit_text(st.delay).trim().parse::<u64>().unwrap_or(0);
        entry.auto_start = get_check(st.auto);
    }

    fn st_load(st: &SettingsState, idx: usize) {
        let Some(entry) = st.config.entries.get(idx) else {
            return;
        };
        set_edit(st.args, &entry.args.join(" "));
        set_edit(
            st.watch,
            &entry
                .watch_seconds
                .map(|w| w.to_string())
                .unwrap_or_default(),
        );
        set_edit(st.delay, &entry.delay_seconds.to_string());
        set_check(st.auto, entry.auto_start);
    }

    fn st_refresh_list(st: &SettingsState) {
        unsafe { SendMessageW(st.list, LB_RESETCONTENT, 0, 0) };
        for entry in &st.config.entries {
            let label = format!(
                "{}{}",
                entry.file_name(),
                if entry.auto_start { " (auto)" } else { "" }
            );
            let w = wide(&label);
            unsafe { SendMessageW(st.list, LB_ADDSTRING, 0, w.as_ptr() as isize) };
        }
        if let Some(sel) = st.selected {
            unsafe { SendMessageW(st.list, LB_SETCURSEL, sel, 0) };
        }
    }

    fn mk_control(
        parent: Hwnd,
        class: &str,
        text: &str,
        style: u32,
        rect: (i32, i32, i32, i32),
        id: usize,
    ) -> Hwnd {
        let (x, y, w, h) = rect;
        let class_w = wide(class);
        let text_w = wide(text);
        let ctrl = unsafe {
            CreateWindowExW(
                0,
                class_w.as_ptr(),
                text_w.as_ptr(),
                0x4000_0000 | 0x1000_0000 | style, // WS_CHILD | WS_VISIBLE
                x,
                y,
                w,
                h,
                parent,
                id as *mut c_void,
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null_mut(),
            )
        };
        if !ctrl.is_null() {
            let font = unsafe { GetStockObject(17) }; // DEFAULT_GUI_FONT
            unsafe { SendMessageW(ctrl, WM_SETFONT_MSG, font as usize, 1) };
        }
        ctrl
    }

    fn open_settings(_parent: Hwnd) {
        let existing = STATE.with(|st| st.borrow().settings_hwnd);
        if !existing.is_null() && unsafe { IsWindow(existing) } != 0 {
            unsafe {
                ShowWindow(existing, 5); // SW_SHOW
                SetForegroundWindow(existing);
            }
            return;
        }
        let config = config::load(&config::config_path());
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class_name = wide("HushRunSettingsWindow");
            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                lpfn_wnd_proc: settings_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: instance,
                h_icon: std::ptr::null_mut(),
                h_cursor: std::ptr::null_mut(),
                hbr_background: 6 as *mut c_void, // COLOR_WINDOW + 1
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: class_name.as_ptr(),
                h_icon_sm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
            let title = wide("HushRun Settings");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                0x10C8_0000, // WS_VISIBLE | WS_CAPTION | WS_SYSMENU
                220,
                140,
                548,
                486,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return;
            }
            STATE.with(|st| st.borrow_mut().settings_hwnd = hwnd);
            let start = mk_control(
                hwnd,
                "BUTTON",
                "Start HushRun with Windows",
                0x3,
                (16, 12, 320, 22),
                SID_START,
            );
            let notif = mk_control(
                hwnd,
                "BUTTON",
                "Show notifications (balloons)",
                0x3,
                (16, 38, 320, 22),
                SID_NOTIF,
            );
            mk_control(
                hwnd,
                "STATIC",
                "Default watch window (seconds):",
                0,
                (16, 70, 215, 20),
                0,
            );
            let gwatch = mk_control(hwnd, "EDIT", "", 0x0080_0080, (236, 68, 56, 23), SID_GWATCH);
            mk_control(hwnd, "STATIC", "Programs:", 0, (16, 102, 240, 20), 0);
            let list = mk_control(
                hwnd,
                "LISTBOX",
                "",
                0x0080_0001 | 0x0020_0000,
                (16, 124, 336, 148),
                SID_LIST,
            );
            mk_control(
                hwnd,
                "BUTTON",
                "Add\u{2026}",
                0,
                (364, 124, 140, 27),
                SID_ADD,
            );
            mk_control(hwnd, "BUTTON", "Remove", 0, (364, 159, 140, 27), SID_REMOVE);
            mk_control(hwnd, "STATIC", "Pick a program to edit its profile below. (auto) entries start hidden with the tray.", 0, (364, 196, 150, 64), 0);
            mk_control(hwnd, "STATIC", "Arguments:", 0, (16, 286, 70, 20), 0);
            let args = mk_control(hwnd, "EDIT", "", 0x0080_0080, (88, 284, 416, 23), SID_ARGS);
            mk_control(hwnd, "STATIC", "Watch s:", 0, (16, 318, 58, 20), 0);
            let watch = mk_control(hwnd, "EDIT", "", 0x0080_0080, (76, 316, 48, 23), SID_WATCH);
            mk_control(hwnd, "STATIC", "Delay s:", 0, (136, 318, 52, 20), 0);
            let delay = mk_control(hwnd, "EDIT", "", 0x0080_0080, (190, 316, 48, 23), SID_DELAY);
            let auto = mk_control(
                hwnd,
                "BUTTON",
                "Run hidden at tray start",
                0x3,
                (252, 316, 252, 22),
                SID_AUTO,
            );
            mk_control(hwnd, "BUTTON", "OK", 0, (308, 404, 92, 30), SID_OK);
            mk_control(hwnd, "BUTTON", "Cancel", 0, (412, 404, 92, 30), SID_CANCEL);
            SETTINGS.with(|s| {
                *s.borrow_mut() = Some(SettingsState {
                    config,
                    selected: None,
                    list,
                    args,
                    watch,
                    delay,
                    auto,
                    start,
                    notif,
                    gwatch,
                });
            });
            SETTINGS.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    set_check(st.start, st.config.start_with_windows);
                    set_check(st.notif, st.config.notifications);
                    set_edit(st.gwatch, &st.config.watch_seconds.to_string());
                    st_refresh_list(st);
                }
            });
        }
    }

    fn settings_save_and_close(hwnd: Hwnd) {
        let cfg = SETTINGS.with(|s| {
            let mut borrow = s.borrow_mut();
            let st = borrow.as_mut()?;
            st_commit(st);
            let mut cfg = st.config.clone();
            cfg.start_with_windows = get_check(st.start);
            cfg.notifications = get_check(st.notif);
            cfg.watch_seconds = edit_text(st.gwatch)
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|v| (1..=3600).contains(v))
                .unwrap_or(15);
            Some(cfg)
        });
        let Some(cfg) = cfg else {
            unsafe { DestroyWindow(hwnd) };
            return;
        };
        if let Err(e) = config::save(&config::config_path(), &cfg) {
            info_box(hwnd, &format!("Could not save settings: {e}"), true);
            return;
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Err(e) = platform::set_autostart(cfg.start_with_windows, &exe) {
                info_box(
                    hwnd,
                    &format!("Settings saved, but starting with Windows could not be updated: {e}"),
                    true,
                );
            }
        }
        let tray_hwnd = STATE.with(|st| st.borrow().hwnd);
        balloon_from_state(tray_hwnd, "Settings saved", false);
        unsafe { DestroyWindow(hwnd) };
    }

    unsafe extern "system" fn settings_proc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize {
        match msg {
            WM_COMMAND_MSG => {
                let id = w & 0xFFFF;
                let notify = (w >> 16) & 0xFFFF;
                match (id, notify) {
                    (SID_LIST, n) if n == LBN_SELCHANGE => {
                        SETTINGS.with(|s| {
                            let mut borrow = s.borrow_mut();
                            if let Some(st) = borrow.as_mut() {
                                st_commit(st);
                                let sel = unsafe { SendMessageW(st.list, LB_GETCURSEL, 0, 0) };
                                if sel >= 0 && (sel as usize) < st.config.entries.len() {
                                    st.selected = Some(sel as usize);
                                    st_load(st, sel as usize);
                                }
                            }
                        });
                    }
                    (SID_ADD, _) => {
                        if let Some(path) = file_dialog(
                            hwnd,
                            false,
                            &["Programs (*.exe)", "*.exe", "All files (*.*)", "*.*"],
                            "",
                        ) {
                            SETTINGS.with(|s| {
                                let mut borrow = s.borrow_mut();
                                if let Some(st) = borrow.as_mut() {
                                    st_commit(st);
                                    st.config.entries.push(ProgramEntry {
                                        program: path,
                                        args: Vec::new(),
                                        watch_seconds: None,
                                        delay_seconds: 0,
                                        auto_start: false,
                                    });
                                    st.selected = Some(st.config.entries.len() - 1);
                                    st_refresh_list(st);
                                    if let Some(sel) = st.selected {
                                        st_load(st, sel);
                                    }
                                }
                            });
                        }
                    }
                    (SID_REMOVE, _) => {
                        SETTINGS.with(|s| {
                            let mut borrow = s.borrow_mut();
                            if let Some(st) = borrow.as_mut() {
                                if let Some(idx) = st.selected.take() {
                                    if idx < st.config.entries.len() {
                                        unsafe { SendMessageW(st.list, LB_DELETESTRING, idx, 0) };
                                        st.config.entries.remove(idx);
                                    }
                                    set_edit(st.args, "");
                                    set_edit(st.watch, "");
                                    set_edit(st.delay, "");
                                    set_check(st.auto, false);
                                }
                            }
                        });
                    }
                    (SID_OK, _) => settings_save_and_close(hwnd),
                    (SID_CANCEL, _) => {
                        unsafe { DestroyWindow(hwnd) };
                    }
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                unsafe { DestroyWindow(hwnd) };
                0
            }
            WM_DESTROY => {
                SETTINGS.with(|s| *s.borrow_mut() = None);
                STATE.with(|st| st.borrow_mut().settings_hwnd = std::ptr::null_mut());
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
        }
    }

    pub fn run() -> Result<String, String> {
        unsafe {
            FreeConsole();
            let mutex_name = wide("HushRunTrayMutex");
            let mutex = CreateMutexW(std::ptr::null_mut(), 1, mutex_name.as_ptr());
            if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                let text = wide("HushRun is already running in the notification area.");
                let caption = wide("HushRun");
                MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    caption.as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
                return Ok("HushRun tray is already running.".to_string());
            }

            // Unclean-exit recovery: a leftover marker file means
            // the previous tray never shut down cleanly (crash,
            // kill, power loss). Offer to restore whatever it left
            // hidden before this instance settles in.
            let marker = session::store_path().with_file_name("tray.running");
            if marker.exists() {
                let leftover = session::load(&session::store_path());
                if !leftover.is_empty() {
                    let text = wide(&format!(
                        "HushRun didn't shut down cleanly last time, and {} program(s) were left hidden. Restore them now?",
                        leftover.len()
                    ));
                    let caption = wide("HushRun");
                    let answer = MessageBoxW(
                        std::ptr::null_mut(),
                        text.as_ptr(),
                        caption.as_ptr(),
                        MB_YESNO | MB_ICONQUESTION,
                    );
                    if answer == IDYES {
                        for s in &leftover {
                            if platform::process_alive(s.pid) {
                                app::show_all(s.pid);
                            }
                        }
                        let _ = session::save(&session::store_path(), &[]);
                    }
                }
            }
            let _ = std::fs::write(&marker, b"running\n");

            let instance = GetModuleHandleW(std::ptr::null());
            let icon_on = create_app_icon(false);
            let icon_off = create_app_icon(true);
            let class_name = wide("HushRunTrayWindow");
            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                lpfn_wnd_proc: wnd_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: instance,
                h_icon: icon_on,
                h_cursor: std::ptr::null_mut(),
                hbr_background: std::ptr::null_mut(),
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: class_name.as_ptr(),
                h_icon_sm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
            let title = wide("HushRun");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                -3isize as Hwnd, // HWND_MESSAGE: message-only window
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err(format!(
                    "could not create the tray window (Windows error {})",
                    GetLastError()
                ));
            }
            let taskbar_name = wide("TaskbarCreated");
            let taskbar_msg = RegisterWindowMessageW(taskbar_name.as_ptr());
            STATE.with(|st| {
                let mut st = st.borrow_mut();
                st.hwnd = hwnd;
                st.icon = icon_off;
                st.icon_on = icon_on;
                st.icon_off = icon_off;
                st.mutex = mutex;
                st.taskbar_msg = taskbar_msg;
            });

            let mut nid = nid_base(hwnd, icon_off);
            nid.u_flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.u_callback_message = TRAY_MSG;
            let tip = wide(&tip_text(0));
            nid.sz_tip[..tip.len()].copy_from_slice(&tip);
            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                return Err("could not add the tray icon".to_string());
            }
            // Bring back what was tucked away across a reboot, and
            // start the auto-start program entries.
            relaunch_dead_sessions();
            run_auto_entries();
            SetTimer(hwnd, 1, 1000, std::ptr::null_mut());
            // Global hotkeys. If another app already owns a combo,
            // registration simply fails and the menu still works.
            RegisterHotKey(
                hwnd,
                HOTKEY_HIDE,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                VK_H,
            );
            RegisterHotKey(
                hwnd,
                HOTKEY_RESTORE,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                VK_U,
            );
            tick(hwnd);

            let mut msg: Msg = std::mem::zeroed();
            loop {
                let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if r <= 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Clean exit: clear the marker so the next launch
            // doesn't offer the recovery prompt.
            let _ = std::fs::remove_file(&marker);
            DestroyIcon(icon_on);
            DestroyIcon(icon_off);
            let held_mutex = STATE.with(|st| st.borrow().mutex);
            if !held_mutex.is_null() {
                CloseHandle(held_mutex);
            }
            Ok("HushRun tray closed.".to_string())
        }
    }
}
