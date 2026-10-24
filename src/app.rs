//! Command implementations: platform calls + session store.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cli::Command;
use crate::config;
use crate::platform;
use crate::session::{self, Session};

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Hide every currently-visible window of `pid`, then keep watching
/// for `watch_secs`, hiding new windows as they appear (many
/// programs create their window a beat after starting). Returns how
/// many distinct windows were hidden.
pub(crate) fn watch_hide(pid: u32, watch_secs: u64) -> usize {
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

pub(crate) fn show_all(pid: u32) -> usize {
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
            let watch_secs =
                watch_secs.unwrap_or_else(|| config::load(&config::config_path()).watch_seconds);
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
        Command::Hide { target } if target.contains('*') => {
            let matches: Vec<(u32, String)> = platform::all_processes()
                .into_iter()
                .filter(|(_, exe)| platform::wildcard_match(&target, exe))
                .collect();
            if matches.is_empty() {
                return Err(format!("no running program matches pattern '{target}'"));
            }
            let mut sessions = session::load(&path);
            let mut lines = Vec::new();
            for (pid, exe) in matches {
                let hidden = watch_hide(pid, 5);
                if hidden > 0 {
                    add_session(
                        &mut sessions,
                        Session {
                            pid,
                            program: exe.clone(),
                            args: Vec::new(),
                            started_unix: now_unix(),
                            windows_hidden: hidden as u32,
                        },
                    );
                    lines.push(format!("  {exe} (pid {pid}) — {hidden} window(s) hidden"));
                }
            }
            session::save(&path, &sessions).map_err(|e| format!("could not save sessions: {e}"))?;
            if lines.is_empty() {
                return Err(format!(
                    "programs matching '{target}' are running but none had a visible window"
                ));
            }
            Ok(format!(
                "Hidden {} program(s):\n{}",
                lines.len(),
                lines.join("\n")
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
        Command::List { json } => {
            let mut sessions = session::load(&path);
            let before = sessions.len();
            sessions.retain(|s| platform::process_alive(s.pid));
            let pruned = before - sessions.len();
            if pruned > 0 {
                let _ = session::save(&path, &sessions);
            }
            if json {
                return Ok(session::render_store(&sessions).trim_end().to_string());
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
                .filter(|s| session_matches(s, &target))
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
        Command::Doctor => Ok(doctor_report()),
        Command::Export { path: dest } => export_settings(std::path::Path::new(&dest)),
        Command::Import { path: source } => import_settings(std::path::Path::new(&source)),
        Command::Tray => crate::tray::run(),
        Command::Help | Command::Version => {
            unreachable!("handled in main")
        }
    }
}

/// Restore matching: a target with `*` is a wildcard pattern on
/// the program's file name; anything else uses the exact rules.
fn session_matches(s: &Session, target: &str) -> bool {
    if target.contains('*') {
        platform::wildcard_match(target, s.file_name())
    } else {
        s.matches(target)
    }
}

/// A plain-language health check of everything HushRun keeps:
/// the data folder, the settings file, every recorded session.
fn doctor_report() -> String {
    let mut out = String::from("HushRun doctor\n");
    let dir = session::data_dir();
    out.push_str(&format!("  data folder: {}\n", dir.display()));
    let cfg_path = config::config_path();
    match std::fs::read_to_string(&cfg_path) {
        Ok(text) => match config::parse_config(&text) {
            Some(cfg) => out.push_str(&format!(
                "  settings: ok ({} program entr{}, watch {}s, notifications {})\n",
                cfg.entries.len(),
                if cfg.entries.len() == 1 { "y" } else { "ies" },
                cfg.watch_seconds,
                if cfg.notifications { "on" } else { "off" }
            )),
            None => out.push_str(
                "  settings: config.json is unreadable — defaults are in use. \
                 Fix or delete it, or re-import known-good settings.\n",
            ),
        },
        Err(_) => out.push_str("  settings: no config.json yet (defaults in use)\n"),
    }
    let marker = session::store_path().with_file_name("tray.running");
    if marker.exists() {
        out.push_str(
            "  tray: a tray marker is present — the tray is running, or a previous run ended uncleanly\n",
        );
    }
    let sessions = session::load(&session::store_path());
    if sessions.is_empty() {
        out.push_str("  sessions: none recorded\n");
    } else {
        out.push_str(&format!("  sessions: {} recorded\n", sessions.len()));
        for s in &sessions {
            if platform::process_alive(s.pid) {
                let windows = platform::windows_of(s.pid);
                let hidden = windows
                    .iter()
                    .filter(|w| !platform::is_window_visible(**w))
                    .count();
                out.push_str(&format!(
                    "    pid {:<7} {} — alive, {} window(s), {} currently hidden\n",
                    s.pid,
                    s.program,
                    windows.len(),
                    hidden
                ));
            } else {
                out.push_str(&format!(
                    "    pid {:<7} {} — EXITED (will be pruned on next list/tray start)\n",
                    s.pid, s.program
                ));
            }
        }
    }
    out.trim_end().to_string()
}

/// Write the current settings to `dest` (used by `export` and the
/// tray's Export menu item).
pub(crate) fn export_settings(dest: &std::path::Path) -> Result<String, String> {
    let cfg = config::load(&config::config_path());
    std::fs::write(dest, config::render_config(&cfg))
        .map_err(|e| format!("could not write {}: {e}", dest.display()))?;
    Ok(format!(
        "Settings written to {} ({} program entries).",
        dest.display(),
        cfg.entries.len()
    ))
}

/// Load settings from `source`, keeping the current file as
/// `config.json.bak` first (used by `import` and the tray).
pub(crate) fn import_settings(source: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(source)
        .map_err(|e| format!("could not read {}: {e}", source.display()))?;
    let cfg = config::parse_config(&text)
        .ok_or_else(|| format!("{} is not a valid HushRun settings file", source.display()))?;
    let path = config::config_path();
    if path.exists() {
        let _ = std::fs::copy(&path, path.with_extension("json.bak"));
    }
    config::save(&path, &cfg).map_err(|e| format!("could not save settings: {e}"))?;
    Ok(format!(
        "Imported settings from {} ({} program entries). If they include a \
         start-with-Windows change, it applies next time settings are saved from the tray.",
        source.display(),
        cfg.entries.len()
    ))
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
