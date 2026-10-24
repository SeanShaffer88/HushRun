//! The platform seam. Everything OS-specific lives behind these
//! functions; the Windows implementation is real Win32 FFI, and a
//! stub for other systems keeps the crate (and its tests) building
//! anywhere.

#[cfg(windows)]
#[path = "windows.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "stub.rs"]
mod imp;

pub use imp::{
    all_processes, find_process_by_name, is_window_visible, launch_hidden, process_alive,
    set_window_visible, windows_of,
};

// Used by the tray (Windows-only); on other platforms only the
// seam's completeness needs them.
#[cfg_attr(not(windows), allow(unused_imports))]
pub use imp::{
    exe_name, foreground_window, own_pid, set_autostart, visible_apps, window_pid, window_title,
};

/// Case-insensitive wildcard match where `*` matches any run of
/// characters (including none) — the bulk hide/restore pattern
/// language. Pure — tested on every platform.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    wildcard_at(&p, &t)
}

fn wildcard_at(pattern: &[char], text: &[char]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern[0] == '*' {
        // Collapse runs of stars; a trailing star matches all.
        let rest = &pattern[pattern.iter().take_while(|&&c| c == '*').count()..];
        if rest.is_empty() {
            return true;
        }
        return (0..=text.len()).any(|i| wildcard_at(rest, &text[i..]));
    }
    if text.is_empty() {
        return false;
    }
    let same = pattern[0].eq_ignore_ascii_case(&text[0])
        || (pattern[0] as u32 > 127 && pattern[0].to_lowercase().eq(text[0].to_lowercase()));
    same && wildcard_at(&pattern[1..], &text[1..])
}

/// Build a Windows-style command line from a program and its
/// arguments, applying the standard quoting rules (an argument is
/// quoted when it is empty or contains a space or a quote;
/// backslashes are doubled where they precede a quote or the
/// closing quote). Pure — tested on every platform.
// Used by the Windows backend; on other platforms only the tests
// exercise it.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn build_command_line(program: &str, args: &[String]) -> String {
    let mut out = quote_arg(program);
    for arg in args {
        out.push(' ');
        out.push_str(&quote_arg(arg));
    }
    out
}

fn quote_arg(arg: &str) -> String {
    // Quote when empty, when the argument contains a space, tab or
    // quote, or when it ends with a backslash (which could otherwise
    // be misread as escaping the closing quote of a neighbour).
    if !arg.is_empty() && !arg.contains([' ', '"', '\t']) && !arg.ends_with('\\') {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(ch);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_args_are_unquoted() {
        assert_eq!(
            build_command_line("notepad.exe", &["a.txt".into()]),
            "notepad.exe a.txt"
        );
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        assert_eq!(
            build_command_line("C:\\Program Files\\Tool\\tool.exe", &[]),
            "\"C:\\Program Files\\Tool\\tool.exe\""
        );
    }

    #[test]
    fn embedded_quotes_and_trailing_backslashes() {
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_arg("C:\\dir\\"), "\"C:\\dir\\\\\"");
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("chrome*", "chrome.exe"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("note*", "Notepad.exe"));
        assert!(wildcard_match("*pad*", "notepad.exe"));
        assert!(wildcard_match("exact", "exact"));
        assert!(!wildcard_match("chrome*", "firefox.exe"));
        assert!(!wildcard_match("note", "notepad.exe"));
        assert!(!wildcard_match("", "x"));
        assert!(wildcard_match("", ""));
    }
}
