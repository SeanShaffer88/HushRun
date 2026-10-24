//! Non-Windows stub: HushRun's job only exists on Windows, but the
//! CLI and session logic still run (and are tested) elsewhere.

pub fn launch_hidden(_program: &str, _args: &[String]) -> Result<u32, String> {
    Err("HushRun can only start programs on Windows".to_string())
}

pub fn windows_of(_pid: u32) -> Vec<isize> {
    Vec::new()
}

pub fn is_window_visible(_hwnd: isize) -> bool {
    false
}

pub fn set_window_visible(_hwnd: isize, _visible: bool) -> bool {
    false
}

pub fn process_alive(_pid: u32) -> bool {
    false
}

pub fn find_process_by_name(_name: &str) -> Option<(u32, String)> {
    None
}
