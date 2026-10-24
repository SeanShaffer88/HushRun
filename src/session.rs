//! Hidden-program sessions and their on-disk store.
//!
//! The store is a single JSON file written atomically (temp file +
//! replace). The JSON reader/writer here is deliberately small: it
//! covers exactly the shapes the store uses, and anything malformed
//! is skipped rather than crashing — a corrupt store must never
//! strand a user's hidden programs without `restore-all` recourse
//! from a fresh state.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pid: u32,
    pub program: String,
    pub args: Vec<String>,
    pub started_unix: u64,
    pub windows_hidden: u32,
}

impl Session {
    /// Does `target` (a pid string or a program name fragment) pick
    /// out this session? Name matching is on the file name only,
    /// case-insensitive, and tolerates a missing `.exe`.
    pub fn matches(&self, target: &str) -> bool {
        if let Ok(pid) = target.parse::<u32>() {
            return pid == self.pid;
        }
        // Split on both separators: session programs are Windows
        // paths regardless of the OS this code is compiled for.
        let file_name = self
            .program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.program)
            .to_lowercase();
        let wanted = target.to_lowercase();
        file_name == wanted || file_name == format!("{wanted}.exe")
    }
}

pub fn store_path() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if !local.is_empty() {
            return Path::new(&local).join("HushRun").join("sessions.json");
        }
    }
    std::env::temp_dir().join("HushRun").join("sessions.json")
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

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn render_store(sessions: &[Session]) -> String {
    let mut out = String::from("[\n");
    for (i, s) in sessions.iter().enumerate() {
        let args: Vec<String> = s.args.iter().map(|a| format!("\"{}\"", esc(a))).collect();
        out.push_str(&format!(
            "  {{\"pid\": {}, \"program\": \"{}\", \"args\": [{}], \"started_unix\": {}, \"windows_hidden\": {}}}{}\n",
            s.pid,
            esc(&s.program),
            args.join(", "),
            s.started_unix,
            s.windows_hidden,
            if i + 1 == sessions.len() { "" } else { "," }
        ));
    }
    out.push_str("]\n");
    out
}

// ---------- parsing (small, tolerant) ----------

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
    Other,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.skip_ws();
        match self.peek()? {
            b'{' => {
                self.pos += 1;
                let mut fields = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        return Some(Value::Obj(fields));
                    }
                    let key = match self.value()? {
                        Value::Str(s) => s,
                        _ => return None,
                    };
                    self.skip_ws();
                    if self.peek() != Some(b':') {
                        return None;
                    }
                    self.pos += 1;
                    let val = self.value()?;
                    fields.push((key, val));
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Some(Value::Obj(fields));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        return Some(Value::Arr(items));
                    }
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Some(Value::Arr(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Value::Str),
            b'-' | b'0'..=b'9' => self.number().map(Value::Num),
            _ => {
                // true / false / null — consumed, not used.
                while matches!(self.peek(), Some(b'a'..=b'z')) {
                    self.pos += 1;
                }
                Some(Value::Other)
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.peek() != Some(b'"') {
            return None;
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let byte = self.peek()?;
            self.pos += 1;
            match byte {
                b'"' => return Some(out),
                b'\\' => {
                    let e = self.peek()?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'u' => {
                            let hex = self.bytes.get(self.pos..self.pos + 4)?;
                            let code =
                                u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                            self.pos += 4;
                        }
                        _ => return None,
                    }
                }
                _ => {
                    // Reassemble UTF-8: collect the full sequence.
                    let start = self.pos - 1;
                    let len = utf8_len(byte);
                    let end = start + len;
                    let slice = self.bytes.get(start..end)?;
                    out.push_str(std::str::from_utf8(slice).ok()?);
                    self.pos = end;
                }
            }
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

fn get<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Num(n) if *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

fn as_str(value: &Value) -> Option<&str> {
    match value {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

pub fn parse_store(text: &str) -> Option<Vec<Session>> {
    let mut parser = Parser::new(text);
    let Value::Arr(items) = parser.value()? else {
        return None;
    };
    let mut sessions = Vec::new();
    for item in items {
        let Value::Obj(fields) = item else { continue };
        let (Some(pid), Some(program)) = (
            get(&fields, "pid").and_then(as_u64),
            get(&fields, "program").and_then(as_str),
        ) else {
            continue;
        };
        let args = match get(&fields, "args") {
            Some(Value::Arr(values)) => values
                .iter()
                .filter_map(as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        sessions.push(Session {
            pid: pid as u32,
            program: program.to_owned(),
            args,
            started_unix: get(&fields, "started_unix").and_then(as_u64).unwrap_or(0),
            windows_hidden: get(&fields, "windows_hidden").and_then(as_u64).unwrap_or(0) as u32,
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
}
