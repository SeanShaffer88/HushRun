//! Non-Windows stub: HushRun's job only exists on Windows, but the
//! CLI and session logic still run (and are tested) elsewhere.

pub fn launch_hidden(_program: &str, _args: &[String]) -> Result<Process, String> {
    Err("HushRun can only start programs on Windows".to_string())
}

pub fn windows_of(_pid: u32) -> Vec<isize> {
    Vec::new()
}

pub fn is_window_visible(_hwnd: isize) -> bool {
    false
}

pub struct Process {
    pub pid: u32,
    pub creation_filetime: u64,
}

impl Process {
    pub fn open(_pid: u32) -> Result<Self, String> {
        Err("Process identity is only available on Windows. No windows changed.".into())
    }
    pub fn verify(&self) -> Result<(), String> {
        super::verify_identity(Some(self.creation_filetime), self.creation_filetime, false)
    }
}

pub fn set_window_visible(_process: &Process, _hwnd: isize, _visible: bool) -> Result<(), String> {
    Err("Window actions are only available on Windows".into())
}

pub fn session_process(_pid: u32, expected: Option<u64>) -> Result<Option<Process>, String> {
    super::verify_identity(expected, 0, false)?;
    Err("Process identity is only available on Windows".into())
}

pub fn process_alive(_pid: u32) -> bool {
    false
}

pub fn find_process_by_name(_name: &str) -> Option<(u32, String)> {
    None
}

#[allow(dead_code)]
pub fn window_title(_hwnd: isize) -> String {
    String::new()
}

pub fn all_processes() -> Vec<(u32, String)> {
    Vec::new()
}

#[allow(dead_code)]
pub fn set_autostart(_enabled: bool, _exe: &std::path::Path) -> Result<(), String> {
    Err("starting with Windows is only available on Windows".to_string())
}

// The next five serve the Windows tray; here they exist so the
// seam is complete on every platform.
#[allow(dead_code)]
pub fn visible_apps() -> Vec<(u32, String)> {
    Vec::new()
}

#[allow(dead_code)]
pub fn foreground_window() -> Option<isize> {
    None
}

#[allow(dead_code)]
pub fn window_pid(_hwnd: isize) -> Option<u32> {
    None
}

#[allow(dead_code)]
pub fn own_pid() -> u32 {
    std::process::id()
}

#[allow(dead_code)]
pub fn exe_name(_pid: u32) -> Option<String> {
    None
}

pub struct SessionGate;
impl SessionGate {
    pub fn acquire(_directory: &std::path::Path) -> Result<Self, String> {
        Err("Session locking is only available on Windows".into())
    }
}
pub fn new_receipt() -> Result<String, String> {
    Err("Session receipts require Windows".into())
}
pub fn checked_windows_of(_pid: u32) -> Result<Vec<isize>, String> {
    Err("Window queries require Windows".into())
}
