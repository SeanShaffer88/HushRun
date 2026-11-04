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
const SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_TIMEOUT: u32 = 258;
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
    fn WaitForSingleObject(h: Handle, milliseconds: u32) -> u32;
    fn GetProcessTimes(
        h: Handle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> Bool;
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

pub fn launch_hidden(program: &str, args: &[String]) -> Result<Process, String> {
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
    }
    Process::from_handle(pi.dw_process_id, OwnedHandle::new(pi.h_process as usize))
        .map_err(|e| format!("Incomplete hidden launch: owned child pid {}; identity unavailable: {e}. No watcher started.", pi.dw_process_id))
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

struct OwnedHandle {
    raw: usize,
    close: fn(usize),
}

impl OwnedHandle {
    fn new(raw: usize) -> Self {
        Self {
            raw,
            close: |raw| {
                unsafe { CloseHandle(raw as Handle) };
            },
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        (self.close)(self.raw);
    }
}

pub struct Process {
    handle: OwnedHandle,
    pub pid: u32,
    pub creation_filetime: u64,
}

impl Process {
    fn from_handle(pid: u32, handle: OwnedHandle) -> Result<Self, String> {
        let creation_filetime = creation_time(&handle)?;
        let process = Self {
            handle,
            pid,
            creation_filetime,
        };
        process.verify()?;
        Ok(process)
    }

    pub fn open(pid: u32) -> Result<Self, String> {
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(format!(
                "Cannot verify pid {pid} (Windows error {}). Action refused.",
                unsafe { GetLastError() }
            ));
        }
        Self::from_handle(pid, OwnedHandle::new(handle as usize))
    }

    pub fn verify(&self) -> Result<(), String> {
        super::verify_identity(
            Some(self.creation_filetime),
            creation_time(&self.handle)?,
            unsafe { WaitForSingleObject(self.handle.raw as Handle, 0) } == WAIT_TIMEOUT,
        )
    }
}

pub fn session_process(pid: u32, expected: Option<u64>) -> Result<Option<Process>, String> {
    let expected =
        expected.ok_or("Unverified session: no process creation identity. Action refused.")?;
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        let code = unsafe { GetLastError() };
        if code == 87 {
            return Ok(None);
        }
        return Err(format!(
            "Cannot verify pid {pid} (Windows error {code}). Action refused."
        ));
    }
    let handle = OwnedHandle::new(raw as usize);
    let actual = creation_time(&handle)?;
    super::verify_identity(Some(expected), actual, true)?;
    match unsafe { WaitForSingleObject(raw, 0) } {
        0 => Ok(None),
        WAIT_TIMEOUT => Ok(Some(Process {
            handle,
            pid,
            creation_filetime: actual,
        })),
        _ => Err("Process lifetime is unknown. Action refused.".into()),
    }
}

fn creation_time(handle: &OwnedHandle) -> Result<u64, String> {
    let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
    if unsafe {
        GetProcessTimes(
            handle.raw as Handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(format!(
            "Process identity is unknown (Windows error {}). Action refused.",
            unsafe { GetLastError() }
        ));
    }
    Ok(creation)
}

pub fn set_window_visible(process: &Process, hwnd: isize, visible: bool) -> Result<(), String> {
    process.verify()?;
    super::verify_window_owner(process.pid, window_pid(hwnd))?;
    let cmd = if visible { SW_SHOW } else { SW_HIDE };
    unsafe { ShowWindow(hwnd as Hwnd, cmd) };
    process.verify()?;
    super::verify_window_owner(process.pid, window_pid(hwnd))?;
    if is_window_visible(hwnd) != visible {
        return Err("Window visibility could not be confirmed; recovery retained.".into());
    }
    Ok(())
}

pub fn process_alive(pid: u32) -> bool {
    Process::open(pid).is_ok()
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

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: Handle,
}

extern "system" {
    fn LockFileEx(
        file: Handle,
        flags: u32,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> Bool;
    fn UnlockFileEx(
        file: Handle,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> Bool;
}
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(algorithm: Handle, buffer: *mut u8, size: u32, flags: u32) -> i32;
}

pub fn new_receipt() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 16, 2) } < 0 {
        return Err("Could not generate session receipt".into());
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub struct SessionGate {
    file: std::fs::File,
}
impl SessionGate {
    pub fn acquire(directory: &std::path::Path) -> Result<Self, String> {
        Self::acquire_for(directory, std::time::Duration::from_secs(2))
    }
    pub(crate) fn acquire_for(
        directory: &std::path::Path,
        timeout: std::time::Duration,
    ) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("sessions.lock"))
            .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let mut overlapped: Overlapped = unsafe { std::mem::zeroed() };
            // Synchronous file, exclusive nonblocking byte zero lock; each attempt owns its handle.
            if unsafe { LockFileEx(file.as_raw_handle(), 3, 0, 1, 0, &mut overlapped) } != 0 {
                return Ok(Self { file });
            }
            let code = unsafe { GetLastError() };
            if code != 33 {
                return Err(format!("Session lock failed (Windows error {code})"));
            }
            if std::time::Instant::now() >= deadline {
                return Err("Session store busy: lock timeout".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
impl Drop for SessionGate {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        let mut overlapped: Overlapped = unsafe { std::mem::zeroed() };
        // Only successfully acquired guards exist; File closes once after this unlock.
        unsafe {
            UnlockFileEx(self.file.as_raw_handle(), 0, 1, 0, &mut overlapped);
        }
    }
}

pub fn checked_windows_of(pid: u32) -> Result<Vec<isize>, String> {
    let mut state = EnumState {
        pid,
        windows: Vec::new(),
    };
    if unsafe { EnumWindows(enum_cb, &mut state as *mut EnumState as isize) } == 0 {
        return Err(format!(
            "Window enumeration failed (Windows error {})",
            unsafe { GetLastError() }
        ));
    }
    Ok(state.windows)
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn owned_handle_closes_once_after_transfer_and_error() {
        static CLOSED: AtomicUsize = AtomicUsize::new(0);
        fn close(_: usize) {
            CLOSED.fetch_add(1, Ordering::SeqCst);
        }
        let handle = OwnedHandle { raw: 123, close };
        let thread = std::thread::spawn(move || {
            let _owned = handle;
            Err::<(), _>("inert query failure")
        });
        assert!(thread.join().unwrap().is_err());
        assert_eq!(CLOSED.load(Ordering::SeqCst), 1);
        let handle = OwnedHandle { raw: 456, close };
        drop(handle);
        assert_eq!(CLOSED.load(Ordering::SeqCst), 2);
    }
}
