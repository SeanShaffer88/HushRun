//! Hidden-program sessions and their on-disk store.
//!
//! The store is a single JSON file written atomically (temp file +
//! replace), using the small shared JSON reader/writer in
//! `crate::json`. Anything malformed is skipped rather than
//! crashing — a corrupt store must never strand a user's hidden
//! programs without `restore-all` recourse from a fresh state.

use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{self, Json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pid: u32,
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

pub fn save(path: &Path, sessions: &[Session]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, render_store(sessions))?;
    fs::rename(&tmp, path)
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
            "  {{\"pid\": {}, \"program\": \"{}\", \"args\": [{}], \"started_unix\": {}, \"windows_hidden\": {}}}{}\n",
            s.pid,
            json::esc(&s.program),
            args.join(", "),
            s.started_unix,
            s.windows_hidden,
            if i + 1 == sessions.len() { "" } else { "," }
        ));
    }
    out.push_str("]\n");
    out
}

// ---------- parsing ----------

pub fn parse_store(text: &str) -> Option<Vec<Session>> {
    let Json::Arr(items) = json::parse(text)? else {
        return None;
    };
    let mut sessions = Vec::new();
    for item in items {
        let Json::Obj(fields) = item else { continue };
        let (Some(pid), Some(program)) = (
            json::get(&fields, "pid").and_then(json::as_u64),
            json::get(&fields, "program").and_then(json::as_str),
        ) else {
            continue;
        };
        sessions.push(Session {
            pid: pid as u32,
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
    fn partial_entries_are_skipped() {
        let text = r#"[{"pid": 7}, {"pid": 8, "program": "a.exe", "args": [], "started_unix": 1, "windows_hidden": 0}]"#;
        let parsed = parse_store(text).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].pid, 8);
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
