//! Command implementations: platform calls + session store.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cli::Command;
use crate::platform;
use crate::session::{self, Session};

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Hide every currently-visible window of `pid`, then keep watching
/// for `watch_secs`, hiding new windows as they appear (many
/// programs create their window a beat after starting). Returns how
/// many distinct windows were hidden.
fn watch_hide(pid: u32, watch_secs: u64) -> usize {
    let mut hidden: HashSet<isize> = HashSet::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(watch_secs);
    loop {
        for hwnd in platform::windows_of(pid) {
            if platform::is_window_visible(hwnd) && platform::set_window_visible(hwnd, false) {
                hidden.insert(hwnd);
            }
        }
        if std::time::Instant::now() >= deadline || !platform::process_alive(pid) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    hidden.len()
}

fn show_all(pid: u32) -> usize {
    let mut shown = 0;
    for hwnd in platform::windows_of(pid) {
        if platform::set_window_visible(hwnd, true) {
            shown += 1;
        }
    }
    shown
}

fn add_session(sessions: &mut Vec<Session>, new: Session) {
    sessions.retain(|s| s.pid != new.pid);
    sessions.push(new);
}

pub fn age_text(started_unix: u64, now: u64) -> String {
    let secs = now.saturating_sub(started_unix);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

pub fn execute(cmd: Command) -> Result<String, String> {
    let path = session::store_path();
    match cmd {
        Command::Run {
            program,
            args,
            watch_secs,
        } => {
            let pid = platform::launch_hidden(&program, &args)?;
            let hidden = watch_hide(pid, watch_secs);
            let mut sessions = session::load(&path);
            add_session(
                &mut sessions,
                Session {
                    pid,
                    program: program.clone(),
                    args,
                    started_unix: now_unix(),
                    windows_hidden: hidden as u32,
                },
            );
            session::save(&path, &sessions).map_err(|e| format!("could not save sessions: {e}"))?;
            Ok(format!(
                "{program} is running hidden (pid {pid}, {hidden} window(s) hidden).\nBring it back with: hushrun restore {pid}"
            ))
        }
        Command::Hide { target } => {
            let (pid, label) = resolve_target(&target)?;
            let hidden = watch_hide(pid, 5);
            let mut sessions = session::load(&path);
            add_session(
                &mut sessions,
                Session {
                    pid,
                    program: label.clone(),
                    args: Vec::new(),
                    started_unix: now_unix(),
                    windows_hidden: hidden as u32,
                },
            );
            session::save(&path, &sessions).map_err(|e| format!("could not save sessions: {e}"))?;
            Ok(format!(
                "{label} is hidden (pid {pid}, {hidden} window(s) hidden).\nBring it back with: hushrun restore {pid}"
            ))
        }
        Command::List => {
            let mut sessions = session::load(&path);
            let before = sessions.len();
            sessions.retain(|s| platform::process_alive(s.pid));
            let pruned = before - sessions.len();
            if pruned > 0 {
                let _ = session::save(&path, &sessions);
            }
            if sessions.is_empty() {
                return Ok("No hidden programs.".to_string());
            }
            let now = now_unix();
            let mut out = String::from("Hidden programs:\n");
            for s in &sessions {
                out.push_str(&format!(
                    "  pid {:<7} {} {} — {} window(s) hidden, started {}\n",
                    s.pid,
                    s.program,
                    if s.args.is_empty() {
                        String::new()
                    } else {
                        s.args.join(" ")
                    },
                    s.windows_hidden,
                    age_text(s.started_unix, now)
                ));
            }
            if pruned > 0 {
                out.push_str(&format!(
                    "({pruned} session(s) for exited programs removed)\n"
                ));
            }
            Ok(out.trim_end().to_string())
        }
        Command::Restore { target } => {
            let mut sessions = session::load(&path);
            let matched: Vec<Session> = sessions
                .iter()
                .filter(|s| s.matches(&target))
                .cloned()
                .collect();
            if matched.is_empty() {
                return Err(format!(
                    "no hidden session matches '{target}'. Try `hushrun list`."
                ));
            }
            let mut lines = Vec::new();
            for s in &matched {
                if platform::process_alive(s.pid) {
                    let shown = show_all(s.pid);
                    lines.push(format!(
                        "{} (pid {}) is back — {shown} window(s) shown.",
                        s.program, s.pid
                    ));
                } else {
                    lines.push(format!(
                        "{} (pid {}) already exited; session removed.",
                        s.program, s.pid
                    ));
                }
            }
            sessions.retain(|s| !matched.iter().any(|m| m.pid == s.pid));
            session::save(&path, &sessions).map_err(|e| format!("could not save sessions: {e}"))?;
            Ok(lines.join("\n"))
        }
        Command::RestoreAll => {
            let sessions = session::load(&path);
            let mut restored = 0;
            for s in &sessions {
                if platform::process_alive(s.pid) {
                    show_all(s.pid);
                    restored += 1;
                }
            }
            session::save(&path, &[]).map_err(|e| format!("could not save sessions: {e}"))?;
            Ok(format!("Brought back {restored} hidden program(s)."))
        }
        Command::Help | Command::Version => {
            unreachable!("handled in main")
        }
    }
}

/// A target is either a pid or a running program's name.
fn resolve_target(target: &str) -> Result<(u32, String), String> {
    if let Ok(pid) = target.parse::<u32>() {
        if platform::process_alive(pid) {
            return Ok((pid, format!("pid {pid}")));
        }
        return Err(format!("no running process with pid {pid}"));
    }
    platform::find_process_by_name(target).ok_or_else(|| {
        format!("no running program matches '{target}'. Give a pid or the program's .exe name.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(age_text(100, 130), "30s ago");
        assert_eq!(age_text(100, 100 + 600), "10m ago");
        assert_eq!(age_text(100, 100 + 7200), "2h ago");
        assert_eq!(age_text(100, 100 + 172_800), "2d ago");
        assert_eq!(age_text(100, 50), "0s ago");
    }
}
