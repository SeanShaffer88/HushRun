//! HushRun's settings: global preferences plus per-program
//! entries (a profile, and optionally an auto-start rule — "run
//! this hidden whenever the tray starts"). Stored as JSON next to
//! the session store, written atomically, parsed tolerantly: a
//! corrupt settings file falls back to defaults, never to a crash.

use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{self, Json};
use crate::session;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramEntry {
    pub program: String,
    pub args: Vec<String>,
    /// Per-program watch window; `None` uses the global default.
    pub watch_seconds: Option<u64>,
    /// Wait this long after the tray starts before launching.
    pub delay_seconds: u64,
    /// Launch hidden whenever the tray starts.
    pub auto_start: bool,
}

impl ProgramEntry {
    // Used by the tray (Windows-only) and the settings editor.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn file_name(&self) -> &str {
        self.program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.program)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub watch_seconds: u64,
    pub start_with_windows: bool,
    pub notifications: bool,
    pub entries: Vec<ProgramEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch_seconds: 15,
            start_with_windows: false,
            notifications: true,
            entries: Vec::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    session::data_dir().join("config.json")
}

pub fn load(path: &Path) -> Config {
    let Ok(text) = fs::read_to_string(path) else {
        return Config::default();
    };
    parse_config(&text).unwrap_or_default()
}

pub fn save(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, render_config(config))?;
    fs::rename(&tmp, path)
}

pub fn render_config(config: &Config) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!(
        "  \"watch_seconds\": {}, \"start_with_windows\": {}, \"notifications\": {},\n  \"entries\": [",
        config.watch_seconds, config.start_with_windows, config.notifications
    ));
    if config.entries.is_empty() {
        out.push_str("]\n}\n");
        return out;
    }
    out.push('\n');
    for (i, e) in config.entries.iter().enumerate() {
        let args: Vec<String> = e
            .args
            .iter()
            .map(|a| format!("\"{}\"", json::esc(a)))
            .collect();
        let watch = match e.watch_seconds {
            Some(w) => w.to_string(),
            None => "null".to_string(),
        };
        out.push_str(&format!(
            "    {{\"program\": \"{}\", \"args\": [{}], \"watch_seconds\": {}, \"delay_seconds\": {}, \"auto_start\": {}}}{}\n",
            json::esc(&e.program),
            args.join(", "),
            watch,
            e.delay_seconds,
            e.auto_start,
            if i + 1 == config.entries.len() { "" } else { "," }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

pub fn parse_config(text: &str) -> Option<Config> {
    let Json::Obj(fields) = json::parse(text)? else {
        return None;
    };
    let mut config = Config::default();
    if let Some(w) = json::get(&fields, "watch_seconds").and_then(json::as_u64) {
        if w > 0 {
            config.watch_seconds = w;
        }
    }
    if let Some(b) = json::get(&fields, "start_with_windows").and_then(json::as_bool) {
        config.start_with_windows = b;
    }
    if let Some(b) = json::get(&fields, "notifications").and_then(json::as_bool) {
        config.notifications = b;
    }
    if let Some(items) = json::get(&fields, "entries").and_then(json::as_arr) {
        for item in items {
            let Json::Obj(ef) = item else { continue };
            let Some(program) = json::get(ef, "program").and_then(json::as_str) else {
                continue;
            };
            config.entries.push(ProgramEntry {
                program: program.to_owned(),
                args: json::get(ef, "args")
                    .map(json::as_str_arr)
                    .unwrap_or_default(),
                watch_seconds: json::get(ef, "watch_seconds").and_then(json::as_u64),
                delay_seconds: json::get(ef, "delay_seconds")
                    .and_then(json::as_u64)
                    .unwrap_or(0),
                auto_start: json::get(ef, "auto_start")
                    .and_then(json::as_bool)
                    .unwrap_or(false),
            });
        }
    }
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        Config {
            watch_seconds: 30,
            start_with_windows: true,
            notifications: false,
            entries: vec![ProgramEntry {
                program: "C:\\Apps\\sync.exe".into(),
                args: vec!["--quiet".into()],
                watch_seconds: Some(45),
                delay_seconds: 10,
                auto_start: true,
            }],
        }
    }

    #[test]
    fn round_trip() {
        let config = sample();
        assert_eq!(parse_config(&render_config(&config)).unwrap(), config);
    }

    #[test]
    fn defaults_on_garbage() {
        assert_eq!(parse_config("nope"), None);
        assert_eq!(
            load(Path::new("/nonexistent/config.json")),
            Config::default()
        );
    }

    #[test]
    fn partial_config_keeps_defaults() {
        let config = parse_config(r#"{"watch_seconds": 60}"#).unwrap();
        assert_eq!(config.watch_seconds, 60);
        assert!(config.notifications);
        assert!(config.entries.is_empty());
    }
}
