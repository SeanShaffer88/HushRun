use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{self, Json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Watching,
    Tracked,
    Restoring,
}
impl State {
    fn name(self) -> &'static str {
        match self {
            Self::Watching => "Watching",
            Self::Tracked => "Tracked",
            Self::Restoring => "Restoring",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pid: u32,
    pub receipt: Option<String>,
    pub state: State,
    pub creation_filetime: Option<u64>,
    pub program: String,
    pub args: Vec<String>,
    pub started_unix: u64,
    pub windows_hidden: u32,
}

impl Session {
    /// The file-name part of the program path (both separators —
    /// session programs are Windows paths regardless of the OS
    /// this code is compiled for).
    pub fn file_name(&self) -> &str {
        self.program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.program)
    }

    /// Does `target` (a pid string or a program name fragment) pick
    /// out this session? Name matching is on the file name only,
    /// case-insensitive, and tolerates a missing `.exe`.
    pub fn matches(&self, target: &str) -> bool {
        if let Ok(pid) = target.parse::<u32>() {
            return pid == self.pid;
        }
        let file_name = self.file_name().to_lowercase();
        let wanted = target.to_lowercase();
        file_name == wanted || file_name == format!("{wanted}.exe")
    }
}

/// Where HushRun keeps its data. Portable mode: a `portable.txt`
/// next to the executable puts everything beside the exe.
/// Otherwise `%LOCALAPPDATA%\HushRun`.
pub fn data_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let portable = exe_dir
        .as_ref()
        .map(|d| d.join("portable.txt").exists())
        .unwrap_or(false);
    resolve_data_dir(portable, exe_dir, std::env::var("LOCALAPPDATA").ok())
}

pub(crate) fn resolve_data_dir(
    portable: bool,
    exe_dir: Option<PathBuf>,
    local_app_data: Option<String>,
) -> PathBuf {
    if portable {
        if let Some(dir) = exe_dir {
            return dir;
        }
    }
    if let Some(local) = local_app_data {
        if !local.is_empty() {
            return Path::new(&local).join("HushRun");
        }
    }
    std::env::temp_dir().join("HushRun")
}

pub fn store_path() -> PathBuf {
    data_dir().join("sessions.json")
}

pub fn load(path: &Path) -> Vec<Session> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_store(&text).unwrap_or_default()
}

fn save(path: &Path, sessions: &[Session]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    use std::io::Write;
    let mut file = fs::File::create(&tmp)?;
    file.write_all(render_store(sessions).as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}

/// A strict reload and every subsequent write share one native cross-process lock.
pub struct Store {
    _gate: crate::platform::SessionGate,
    path: PathBuf,
    pub sessions: Vec<Session>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let directory = path.parent().ok_or("Session path has no directory")?;
        let gate = crate::platform::SessionGate::acquire(directory)?;
        let sessions = load_strict(path)?;
        Ok(Self {
            _gate: gate,
            path: path.into(),
            sessions,
        })
    }
    pub fn commit(&self) -> Result<(), String> {
        if parse_store(&render_store(&self.sessions)).as_ref() != Some(&self.sessions) {
            return Err("Invalid session candidate; refusing mutation".into());
        }
        save(&self.path, &self.sessions).map_err(|e| format!("Could not save sessions: {e}"))
    }
    pub fn matching(&self, captured: &Session) -> Option<usize> {
        self.sessions
            .iter()
            .position(|s| same_operation(s, captured))
    }
}

pub fn same_operation(a: &Session, b: &Session) -> bool {
    a.pid == b.pid && a.creation_filetime == b.creation_filetime && a.receipt == b.receipt
}

pub fn load_strict(path: &Path) -> Result<Vec<Session>, String> {
    match fs::read_to_string(path) {
        Ok(text) => {
            parse_store(&text).ok_or_else(|| "Invalid session store; refusing mutation".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Could not read sessions: {e}")),
    }
}

// ---------- rendering ----------

pub fn render_store(sessions: &[Session]) -> String {
    let mut out = String::from("[\n");
    for (i, s) in sessions.iter().enumerate() {
        let args: Vec<String> = s
            .args
            .iter()
            .map(|a| format!("\"{}\"", json::esc(a)))
            .collect();
        out.push_str(&format!(
            "  {{\"pid\": {}, \"program\": \"{}\", \"args\": [{}], \"started_unix\": {}, \"windows_hidden\": {}, \"creation_filetime\": {}, \"receipt\": {}, \"state\": \"{}\"}}{}\n",
            s.pid,
            json::esc(&s.program),
            args.join(", "),
            s.started_unix,
            s.windows_hidden,
            s.creation_filetime.map(|v| format!("\"{v}\"")).unwrap_or_else(|| "null".into()),
            s.receipt.as_ref().map(|v| format!("\"{}\"", json::esc(v))).unwrap_or_else(|| "null".into()),
            s.state.name(),
            if i + 1 == sessions.len() { "" } else { "," }
        ));
    }
    out.push_str("]\n");
    out
}

// ---------- parsing ----------

pub fn parse_store(text: &str) -> Option<Vec<Session>> {
    let Json::Arr(items) = json::parse_complete(text)? else {
        return None;
    };
    let mut sessions = Vec::new();
    for item in items {
        let Json::Obj(fields) = item else { return None };
        let (Some(pid), Some(program)) = (
            json::get(&fields, "pid").and_then(|v| match v {
                Json::Num(n)
                    if n.is_finite() && *n >= 1.0 && *n <= u32::MAX as f64 && n.fract() == 0.0 =>
                {
                    Some(*n as u64)
                }
                _ => None,
            }),
            json::get(&fields, "program").and_then(json::as_str),
        ) else {
            return None;
        };
        if sessions.iter().any(|s: &Session| s.pid == pid as u32) {
            return None;
        }
        sessions.push(Session {
            pid: u32::try_from(pid).ok()?,
            receipt: match json::get(&fields, "receipt") {
                None | Some(Json::Other) => None,
                Some(value) => {
                    let value = json::as_str(value)?;
                    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return None;
                    }
                    Some(value.to_owned())
                }
            },
            state: match json::get(&fields, "state") {
                None => State::Tracked,
                Some(Json::Str(v)) if v == "Watching" => State::Watching,
                Some(Json::Str(v)) if v == "Tracked" => State::Tracked,
                Some(Json::Str(v)) if v == "Restoring" => State::Restoring,
                _ => return None,
            },
            creation_filetime: json::get(&fields, "creation_filetime")
                .and_then(json::as_str)
                .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|v| v.parse().ok()),
            program: program.to_owned(),
            args: json::get(&fields, "args")
                .map(json::as_str_arr)
                .unwrap_or_default(),
            started_unix: json::get(&fields, "started_unix")
                .and_then(json::as_u64)
                .unwrap_or(0),
            windows_hidden: json::get(&fields, "windows_hidden")
                .and_then(json::as_u64)
                .unwrap_or(0) as u32,
        });
    }
    Some(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            pid: 4321,
            receipt: Some("0123456789abcdef0123456789abcdef".into()),
            state: State::Tracked,
            creation_filetime: Some(u64::MAX),
            program: "C:\\Program Files\\Tool\\tool.exe".into(),
            args: vec!["--flag".into(), "sp ace".into(), "quo\"te".into()],
            started_unix: 1_790_000_000,
            windows_hidden: 2,
        }
    }

    #[test]
    fn round_trip() {
        let store = vec![sample()];
        let text = render_store(&store);
        assert_eq!(parse_store(&text).unwrap(), store);
    }

    #[test]
    fn corrupt_store_is_empty_not_a_panic() {
        assert_eq!(parse_store("not json").unwrap_or_default(), Vec::new());
        assert_eq!(load(Path::new("/nonexistent/sessions.json")), Vec::new());
    }

    #[test]
    fn partial_entries_refuse_mutation() {
        let text = r#"[{"pid": 7}, {"pid": 8, "program": "a.exe", "args": [], "started_unix": 1, "windows_hidden": 0}]"#;
        assert!(parse_store(text).is_none());
    }

    #[test]
    fn legacy_and_malformed_identity_remain_unverified() {
        for identity in [
            "",
            r#", "creation_filetime": null"#,
            r#", "creation_filetime": 123"#,
            r#", "creation_filetime": "18446744073709551616""#,
            r#", "creation_filetime": "-1""#,
            r#", "creation_filetime": "+1""#,
            r#", "creation_filetime": "bad""#,
        ] {
            let text = format!(r#"[{{"pid": 8, "program": "fixture.exe"{identity}}}]"#);
            let sessions = parse_store(&text).unwrap();
            assert_eq!(sessions[0].creation_filetime, None);
            assert_eq!(parse_store(&render_store(&sessions)).unwrap(), sessions);
        }
    }

    #[test]
    fn matching() {
        let s = sample();
        assert!(s.matches("4321"));
        assert!(s.matches("tool"));
        assert!(s.matches("TOOL.EXE"));
        assert!(!s.matches("other"));
        assert!(!s.matches("9999"));
    }

    #[test]
    fn data_dir_modes() {
        let exe = PathBuf::from("D:\\Tools\\HushRun");
        assert_eq!(
            resolve_data_dir(true, Some(exe.clone()), Some("C:\\LAD".into())),
            exe
        );
        assert_eq!(
            resolve_data_dir(false, Some(exe), Some("C:\\LAD".into())),
            Path::new("C:\\LAD").join("HushRun")
        );
    }
}

#[cfg(all(test, windows))]
mod coordination_tests {
    use super::*;
    use crate::platform;
    use std::time::Duration;

    fn directory() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("hushrun-watch-b2-resume-tests")
            .join(platform::new_receipt().unwrap())
    }
    fn child_command(directory: &Path, mode: &str) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "session::coordination_tests::protocol_child",
                "--nocapture",
            ])
            .env("HUSHRUN_TEST_GATE_DIR", directory)
            .env("HUSHRUN_TEST_GATE_MODE", mode);
        command
    }
    #[test]
    fn protocol_child() {
        let Some(directory) = std::env::var_os("HUSHRUN_TEST_GATE_DIR") else {
            return;
        };
        let directory = PathBuf::from(directory);
        match std::env::var("HUSHRUN_TEST_GATE_MODE").unwrap().as_str() {
            "write" => {
                let mut store = Store::open(&directory.join("sessions.json")).unwrap();
                store.sessions.push(Session {
                    pid: std::process::id(),
                    receipt: Some(platform::new_receipt().unwrap()),
                    state: State::Tracked,
                    creation_filetime: None,
                    program: "inert child".into(),
                    args: vec![],
                    started_unix: 0,
                    windows_hidden: 0,
                });
                store.commit().unwrap();
            }
            "timeout" => {
                assert!(
                    platform::SessionGate::acquire_for(&directory, Duration::from_millis(60))
                        .err()
                        .unwrap()
                        .contains("timeout")
                );
            }
            "exit-owner" => {
                let gate = platform::SessionGate::acquire(&directory).unwrap();
                // Emulate abrupt owner exit without terminating any other process.
                std::mem::forget(gate);
                std::process::exit(0);
            }
            _ => panic!("unexpected child mode"),
        }
    }
    #[test]
    fn cross_process_writers_reload_instead_of_overwriting() {
        let directory = directory();
        let mut children: Vec<_> = (0..4)
            .map(|_| child_command(&directory, "write").spawn().unwrap())
            .collect();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let sessions = load_strict(&directory.join("sessions.json")).unwrap();
        assert_eq!(sessions.len(), 4);
        let receipts: std::collections::HashSet<_> = sessions.iter().map(|s| &s.receipt).collect();
        assert_eq!(receipts.len(), 4);
    }
    #[test]
    fn native_gate_timeout_unwind_and_owner_exit_release() {
        let directory = directory();
        let gate = platform::SessionGate::acquire(&directory).unwrap();
        assert!(platform::SessionGate::acquire_for(&directory, Duration::from_millis(20)).is_err());
        assert!(child_command(&directory, "timeout")
            .status()
            .unwrap()
            .success());
        drop(gate);
        assert!(std::panic::catch_unwind(|| {
            let _gate = platform::SessionGate::acquire(&directory).unwrap();
            panic!("inert owner unwind");
        })
        .is_err());
        drop(platform::SessionGate::acquire(&directory).unwrap());
        assert!(child_command(&directory, "exit-owner")
            .status()
            .unwrap()
            .success());
        drop(platform::SessionGate::acquire(&directory).unwrap());
        assert!(directory.join("sessions.lock").is_file());
    }
    #[test]
    fn corrupt_or_denied_store_is_never_empty_writeable() {
        let directory = directory();
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sessions.json");
        for invalid in [
            "broken",
            "[] garbage",
            "[{},]",
            "[{}]",
            "[{\"pid\":1,\"pid\":2,\"program\":\"a\"}]",
            "[{\"pid\":1.5,\"program\":\"a\"}]",
            "[{\"pid\":1,\"program\":\"a\",\"state\":1}]",
            "[{\"pid\":1,\"program\":\"a\",\"state\":\"Unknown\"}]",
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(Store::open(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(Store::open(&path).is_err());
    }
}
