//! A deliberately small, tolerant JSON reader/writer shared by
//! the session store and the settings file. It covers exactly the
//! shapes HushRun writes; anything malformed is skipped rather
//! than crashing — a corrupt file must never strand the user.

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    Bool(bool),
    Other,
}

pub(crate) fn parse(text: &str) -> Option<Json> {
    Parser::new(text).value()
}

/// Session mutations require a complete document, never a valid prefix of corrupt data.
pub(crate) fn parse_complete(text: &str) -> Option<Json> {
    let mut parser = Parser::new(text);
    parser.strict = true;
    let value = parser.value()?;
    parser.skip_ws();
    (parser.pos == parser.bytes.len()).then_some(value)
}

pub(crate) fn get<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

pub(crate) fn as_u64(value: &Json) -> Option<u64> {
    match value {
        Json::Num(n) if *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

pub(crate) fn as_str(value: &Json) -> Option<&str> {
    match value {
        Json::Str(s) => Some(s),
        _ => None,
    }
}

pub(crate) fn as_bool(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

pub(crate) fn as_arr(value: &Json) -> Option<&Vec<Json>> {
    match value {
        Json::Arr(items) => Some(items),
        _ => None,
    }
}

pub(crate) fn as_str_arr(value: &Json) -> Vec<String> {
    match as_arr(value) {
        Some(items) => items.iter().filter_map(as_str).map(str::to_owned).collect(),
        None => Vec::new(),
    }
}

/// Escape a string for inclusion inside JSON quotes.
pub(crate) fn esc(text: &str) -> String {
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

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    strict: bool,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
            strict: false,
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

    fn value(&mut self) -> Option<Json> {
        self.skip_ws();
        match self.peek()? {
            b'{' => {
                self.pos += 1;
                let mut fields = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        return Some(Json::Obj(fields));
                    }
                    let key = match self.value()? {
                        Json::Str(s) => s,
                        _ => return None,
                    };
                    self.skip_ws();
                    if self.peek() != Some(b':') {
                        return None;
                    }
                    self.pos += 1;
                    let val = self.value()?;
                    if self.strict && fields.iter().any(|(existing, _)| existing == &key) {
                        return None;
                    }
                    fields.push((key, val));
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.pos += 1;
                            self.skip_ws();
                            if self.strict && matches!(self.peek(), Some(b'}' | b']')) {
                                return None;
                            }
                        }
                        Some(b'}') => {
                            self.pos += 1;
                            return Some(Json::Obj(fields));
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
                        return Some(Json::Arr(items));
                    }
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.peek() {
                        Some(b',') => {
                            self.pos += 1;
                            self.skip_ws();
                            if self.strict && matches!(self.peek(), Some(b'}' | b']')) {
                                return None;
                            }
                        }
                        Some(b']') => {
                            self.pos += 1;
                            return Some(Json::Arr(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            b'-' | b'0'..=b'9' => self.number().map(Json::Num),
            _ => {
                // true / false / null.
                let start = self.pos;
                while matches!(self.peek(), Some(b'a'..=b'z')) {
                    self.pos += 1;
                }
                match &self.bytes[start..self.pos] {
                    b"true" => Some(Json::Bool(true)),
                    b"false" => Some(Json::Bool(false)),
                    b"null" => Some(Json::Other),
                    _ => None,
                }
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
                    if self.strict && byte < 0x20 {
                        return None;
                    }
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
