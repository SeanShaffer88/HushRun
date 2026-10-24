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

#[cfg(windows)]
pub fn run() -> Result<String, String> {
    windows::run()
}

#[cfg(windows)]
mod windows {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use crate::app;
    use crate::platform;
    use crate::session::{self, Session};

    type Hwnd = *mut c_void;
    type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

    const WM_CLOSE: u32 = 0x0010;
    const WM_DESTROY: u32 = 0x0002;
    const WM_TIMER: u32 = 0x0113;
    const WM_NULL: u32 = 0x0000;
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

    const MF_STRING: u32 = 0x0000;
    const MF_GRAYED: u32 = 0x0001;
    const MF_DISABLED: u32 = 0x0002;
    const MF_SEPARATOR: u32 = 0x0800;
    const TPM_RETURNCMD: u32 = 0x0100;
    const TPM_RIGHTBUTTON: u32 = 0x0002;

    const ID_RESTORE_BASE: u32 = 1000;
    const ID_RUN: u32 = 2000;
    const ID_RESTORE_ALL: u32 = 2001;
    const ID_EXIT: u32 = 2002;

    const OFN_PATHMUSTEXIST: u32 = 0x0800;
    const OFN_FILEMUSTEXIST: u32 = 0x1000;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const MB_OK: u32 = 0x0;
    const MB_ICONINFORMATION: u32 = 0x40;
    const MB_ICONERROR: u32 = 0x10;

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
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *const NotifyIconDataW) -> i32;
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
        menu_sessions: Vec<Session>,
        tip_count: usize,
    }

    thread_local! {
        static STATE: RefCell<TrayState> = const { RefCell::new(TrayState {
            hwnd: std::ptr::null_mut(),
            icon: std::ptr::null_mut(),
            menu_sessions: Vec::new(),
            tip_count: usize::MAX,
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

    fn set_tip(hwnd: Hwnd, icon: *mut c_void, count: usize) {
        let mut nid = nid_base(hwnd, icon);
        nid.u_flags = NIF_TIP;
        let text = wide(&tip_text(count));
        let len = text.len().min(127);
        nid.sz_tip[..len].copy_from_slice(&text[..len]);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
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
        for mut s in sessions.drain(..) {
            if !platform::process_alive(s.pid) {
                changed = true;
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
        let icon = STATE.with(|st| {
            let mut st = st.borrow_mut();
            if st.tip_count != count {
                st.tip_count = count;
                return st.icon;
            }
            std::ptr::null_mut()
        });
        if !icon.is_null() {
            set_tip(hwnd, icon, count);
        }
    }

    fn restore_session(s: &Session) {
        app::show_all(s.pid);
        let path = session::store_path();
        let mut sessions = session::load(&path);
        sessions.retain(|x| x.pid != s.pid);
        let _ = session::save(&path, &sessions);
    }

    fn restore_everything() {
        for s in live_sessions() {
            app::show_all(s.pid);
        }
        let _ = session::save(&session::store_path(), &[]);
    }

    fn show_menu(hwnd: Hwnd) {
        let sessions = live_sessions();
        STATE.with(|st| st.borrow_mut().menu_sessions = sessions.clone());
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
        for (i, s) in sessions.iter().enumerate() {
            append(
                MF_STRING,
                (ID_RESTORE_BASE as usize) + i,
                &format!("Restore {} (pid {})", display_name(&s.program), s.pid),
            );
        }
        if !sessions.is_empty() {
            append(MF_STRING, ID_RESTORE_ALL as usize, "Restore all");
            append(MF_SEPARATOR, 0, "");
        }
        append(MF_STRING, ID_RUN as usize, "Run a program hidden\u{2026}");
        append(MF_SEPARATOR, 0, "");
        append(MF_STRING, ID_EXIT as usize, "Exit (restore all)");

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
            ID_RESTORE_ALL => restore_everything(),
            ID_RUN => pick_and_run(hwnd),
            id if id >= ID_RESTORE_BASE => {
                let idx = (id - ID_RESTORE_BASE) as usize;
                let target = STATE.with(|st| st.borrow().menu_sessions.get(idx).cloned());
                if let Some(s) = target {
                    restore_session(&s);
                }
            }
            _ => {}
        }
    }

    fn pick_and_run(hwnd: Hwnd) {
        let filter_parts = ["Programs (*.exe)", "*.exe", "All files (*.*)", "*.*"];
        let mut filter: Vec<u16> = Vec::new();
        for part in filter_parts {
            filter.extend(part.encode_utf16());
            filter.push(0);
        }
        filter.push(0);
        let mut buffer = vec![0u16; 520];
        let mut ofn: OpenFileNameW = unsafe { std::mem::zeroed() };
        ofn.l_struct_size = std::mem::size_of::<OpenFileNameW>() as u32;
        ofn.hwnd_owner = hwnd;
        ofn.lpstr_filter = filter.as_ptr();
        ofn.lpstr_file = buffer.as_mut_ptr();
        ofn.n_max_file = buffer.len() as u32;
        ofn.flags = OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST;
        let ok = unsafe { GetOpenFileNameW(&mut ofn) };
        if ok == 0 {
            return; // cancelled or closed — nothing to do
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(0);
        let path = String::from_utf16_lossy(&buffer[..len]);
        let pid = match platform::launch_hidden(&path, &[]) {
            Ok(pid) => pid,
            Err(message) => {
                let text = wide(&message);
                let caption = wide("HushRun");
                unsafe { MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR) };
                return;
            }
        };
        let store = session::store_path();
        let mut sessions = session::load(&store);
        sessions.retain(|s| s.pid != pid);
        sessions.push(Session {
            pid,
            program: path,
            args: Vec::new(),
            started_unix: app::now_unix(),
            windows_hidden: 0,
        });
        let _ = session::save(&store, &sessions);
        std::thread::spawn(move || {
            let hidden = app::watch_hide(pid, 15);
            let store = session::store_path();
            let mut sessions = session::load(&store);
            if let Some(s) = sessions.iter_mut().find(|s| s.pid == pid) {
                s.windows_hidden = hidden as u32;
                let _ = session::save(&store, &sessions);
            }
        });
    }

    /// The tray icon: a near-black rounded square with a red border
    /// and a white pixel "H" — drawn into a bitmap, no resources.
    fn create_app_icon() -> *mut c_void {
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
                let mut col: u32 = if edge { 0xFFE5_484D } else { 0xFF14_1414 };
                if (8..23).contains(&x) && (5..26).contains(&y) {
                    let (gx, gy) = ((x - 8) / 3, (y - 5) / 3);
                    if (GLYPH[gy as usize] >> (4 - gx)) & 1 == 1 {
                        col = 0xFFF5_F5F5;
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

    unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize {
        match msg {
            TRAY_MSG => {
                if matches!(l as u32, WM_RBUTTONUP | WM_LBUTTONUP | WM_LBUTTONDBLCLK) {
                    show_menu(hwnd);
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
                    Shell_NotifyIconW(NIM_DELETE, &nid);
                    PostQuitMessage(0);
                }
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

            let instance = GetModuleHandleW(std::ptr::null());
            let icon = create_app_icon();
            let class_name = wide("HushRunTrayWindow");
            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                lpfn_wnd_proc: wnd_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: instance,
                h_icon: icon,
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
            STATE.with(|st| {
                let mut st = st.borrow_mut();
                st.hwnd = hwnd;
                st.icon = icon;
            });

            let mut nid = nid_base(hwnd, icon);
            nid.u_flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.u_callback_message = TRAY_MSG;
            let tip = wide(&tip_text(0));
            nid.sz_tip[..tip.len()].copy_from_slice(&tip);
            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                return Err("could not add the tray icon".to_string());
            }
            SetTimer(hwnd, 1, 1000, std::ptr::null_mut());
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
            DestroyIcon(icon);
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            Ok("HushRun tray closed.".to_string())
        }
    }
}
