//! Windows implementation: direct Win32 FFI, no external crates.
//! All `unsafe` is confined to this module; each call is a
//! documented API used with its documented contract.

use std::ffi::c_void;

use super::build_command_line;

type Hwnd = *mut c_void;
type Handle = *mut c_void;
type Bool = i32;

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const STARTF_USESHOWWINDOW: u32 = 0x0000_0001;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const STILL_ACTIVE: u32 = 259;
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const MAX_PATH: usize = 260;

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: Handle,
    h_std_output: Handle,
    h_std_error: Handle,
}

#[repr(C)]
struct ProcessInformation {
    h_process: Handle,
    h_thread: Handle,
    dw_process_id: u32,
    dw_thread_id: u32,
}

#[repr(C)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; MAX_PATH],
}

extern "system" {
    fn CreateProcessW(
        lp_application_name: *const u16,
        lp_command_line: *mut u16,
        lp_process_attributes: *mut c_void,
        lp_thread_attributes: *mut c_void,
        b_inherit_handles: Bool,
        dw_creation_flags: u32,
        lp_environment: *mut c_void,
        lp_current_directory: *const u16,
        lp_startup_info: *mut StartupInfoW,
        lp_process_information: *mut ProcessInformation,
    ) -> Bool;
    fn GetLastError() -> u32;
    fn CloseHandle(h: Handle) -> Bool;
    fn OpenProcess(access: u32, inherit: Bool, pid: u32) -> Handle;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> Bool;
    fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> Bool, param: isize) -> Bool;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> Bool;
    fn IsWindowVisible(hwnd: Hwnd) -> Bool;
    fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, max: i32) -> i32;
    fn GetForegroundWindow() -> Hwnd;
    fn GetCurrentProcessId() -> u32;
}

// The registry API lives in advapi32 — nothing else in the crate
// links it, so these get their own block with the link attribute
// (without it the release link fails with unresolved externals).
#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(
        key: Handle,
        sub_key: *const u16,
        reserved: u32,
        class: *mut u16,
        options: u32,
        sam: u32,
        attrs: *mut c_void,
        out_key: *mut Handle,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(
        key: Handle,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        size: u32,
    ) -> i32;
    fn RegDeleteValueW(key: Handle, name: *const u16) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(h: Handle, entry: *mut ProcessEntry32W) -> Bool;
    fn Process32NextW(h: Handle, entry: *mut ProcessEntry32W) -> Bool;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn launch_hidden(program: &str, args: &[String]) -> Result<u32, String> {
    let mut cmdline = wide(&build_command_line(program, args));
    let mut si: StartupInfoW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<StartupInfoW>() as u32;
    si.dw_flags = STARTF_USESHOWWINDOW;
    si.w_show_window = SW_HIDE as u16;
    let mut pi: ProcessInformation = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmdline.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut si,
            &mut pi,
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(format!(
            "could not start '{program}' (Windows error {code}). Check the path is correct and the program exists."
        ));
    }
    unsafe {
        CloseHandle(pi.h_thread);
        CloseHandle(pi.h_process);
    }
    Ok(pi.dw_process_id)
}

struct EnumState {
    pid: u32,
    windows: Vec<isize>,
}

extern "system" fn enum_cb(hwnd: Hwnd, param: isize) -> Bool {
    let state = unsafe { &mut *(param as *mut EnumState) };
    let mut owner: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
    if owner == state.pid {
        state.windows.push(hwnd as isize);
    }
    1
}

pub fn windows_of(pid: u32) -> Vec<isize> {
    let mut state = EnumState {
        pid,
        windows: Vec::new(),
    };
    unsafe {
        EnumWindows(enum_cb, &mut state as *mut EnumState as isize);
    }
    state.windows
}

pub fn is_window_visible(hwnd: isize) -> bool {
    let visible = unsafe { IsWindowVisible(hwnd as Hwnd) };
    visible != 0
}

pub fn set_window_visible(hwnd: isize, visible: bool) -> bool {
    let cmd = if visible { SW_SHOW } else { SW_HIDE };
    unsafe { ShowWindow(hwnd as Hwnd, cmd) };
    true
}

pub fn process_alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE
    }
}

struct AppsState {
    apps: Vec<(u32, String)>,
}

extern "system" fn apps_cb(hwnd: Hwnd, param: isize) -> Bool {
    let state = unsafe { &mut *(param as *mut AppsState) };
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return 1;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != 0 && !state.apps.iter().any(|(p, _)| *p == pid) {
            let title = String::from_utf16_lossy(&buf[..len as usize]);
            state.apps.push((pid, title));
        }
    }
    1
}

/// Processes that currently own at least one visible, titled
/// top-level window — `(pid, window title)`, one entry per
/// process, in enumeration order. The tray's "hide a running
/// program" list and the foreground-hide hotkey are built on it.
pub fn visible_apps() -> Vec<(u32, String)> {
    let mut state = AppsState { apps: Vec::new() };
    unsafe {
        EnumWindows(apps_cb, &mut state as *mut AppsState as isize);
    }
    state.apps
}

pub fn foreground_window() -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        None
    } else {
        Some(hwnd as isize)
    }
}

pub fn window_pid(hwnd: isize) -> Option<u32> {
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd as Hwnd, &mut pid) };
    if pid == 0 {
        None
    } else {
        Some(pid)
    }
}

pub fn own_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// The executable file name of a running process, if the process
/// list can see it.
pub fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return None;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut found = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32_process_id == pid {
                    let len = entry
                        .sz_exe_file
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(MAX_PATH);
                    found = Some(String::from_utf16_lossy(&entry.sz_exe_file[..len]));
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found
    }
}

/// The title text of a window ("" when it has none).
pub fn window_title(hwnd: isize) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd as Hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

/// Every process the toolhelp snapshot can see: (pid, exe name).
pub fn all_processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return out;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .sz_exe_file
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(MAX_PATH);
                out.push((
                    entry.th32_process_id,
                    String::from_utf16_lossy(&entry.sz_exe_file[..len]),
                ));
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }
    out
}

const HKEY_CURRENT_USER: Handle = 0x8000_0001usize as Handle;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_SZ: u32 = 1;

/// Add or remove HushRun's per-user Run entry (what makes the
/// tray start with Windows). HKCU only — never needs elevation.
pub fn set_autostart(enabled: bool, exe: &std::path::Path) -> Result<(), String> {
    let sub = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = wide("HushRun");
    unsafe {
        let mut key: Handle = std::ptr::null_mut();
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            std::ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            std::ptr::null_mut(),
            &mut key,
            std::ptr::null_mut(),
        );
        if rc != 0 {
            return Err(format!("could not open the Run key (error {rc})"));
        }
        let result = if enabled {
            let value = wide(&format!("\"{}\" tray", exe.to_string_lossy()));
            let rc = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                (value.len() * 2) as u32,
            );
            if rc == 0 {
                Ok(())
            } else {
                Err(format!("could not write the Run entry (error {rc})"))
            }
        } else {
            let rc = RegDeleteValueW(key, name.as_ptr());
            if rc == 0 || rc == 2 {
                Ok(()) // 2 = value already absent
            } else {
                Err(format!("could not remove the Run entry (error {rc})"))
            }
        };
        RegCloseKey(key);
        result
    }
}

pub fn find_process_by_name(name: &str) -> Option<(u32, String)> {
    let wanted = name.to_lowercase();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return None;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut found = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .sz_exe_file
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(MAX_PATH);
                let exe = String::from_utf16_lossy(&entry.sz_exe_file[..len]);
                let lower = exe.to_lowercase();
                if lower == wanted || lower == format!("{wanted}.exe") {
                    found = Some((entry.th32_process_id, exe));
                    break;
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        found
    }
}
