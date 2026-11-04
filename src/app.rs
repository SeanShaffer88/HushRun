//! Command implementations: platform calls + session store.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cli::Command;
use crate::config;
use crate::platform;
use crate::session::{self, Session, State, Store};

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
pub(crate) fn verified_session(s: &Session) -> Result<platform::Process, String> {
    platform::session_process(s.pid, s.creation_filetime)?
        .ok_or_else(|| format!("Process {} exited. Action refused.", s.pid))
}

pub(crate) fn on_worker<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| scope.spawn(f).join().expect("session worker panicked"))
}

pub(crate) fn process_for_hide(pid: u32) -> Result<platform::Process, String> {
    on_worker(|| {
        let store = Store::open(&session::store_path())?;
        if let Some(s) = store.sessions.iter().find(|s| s.pid == pid) {
            return verified_session(s);
        }
        platform::Process::open(pid)
    })
}

fn registration(
    process: &platform::Process,
    program: &str,
    args: &[String],
) -> Result<Session, String> {
    process.verify()?;
    Ok(Session {
        pid: process.pid,
        creation_filetime: Some(process.creation_filetime),
        receipt: Some(platform::new_receipt()?),
        state: State::Watching,
        program: program.into(),
        args: args.into(),
        started_unix: now_unix(),
        windows_hidden: 0,
    })
}

fn begin_at(
    path: &std::path::Path,
    process: &platform::Process,
    program: &str,
    args: &[String],
) -> Result<Session, String> {
    let mut store = Store::open(path)?;
    if let Some(old) = store.sessions.iter().find(|s| s.pid == process.pid) {
        verified_session(old)?;
    }
    let captured = registration(process, program, args)?;
    store.sessions.retain(|s| s.pid != captured.pid);
    store.sessions.push(captured.clone());
    store.commit()?;
    Ok(captured)
}

pub(crate) fn begin_tracking(
    process: &platform::Process,
    program: &str,
    args: &[String],
) -> Result<Session, String> {
    on_worker(|| begin_at(&session::store_path(), process, program, args)).map_err(|e| {
        format!(
            "Incomplete tracking: owned child pid {}, creation {}; {e}. No watcher started.",
            process.pid, process.creation_filetime
        )
    })
}

/// Explicitly migrate only a verified legacy record under the gate.
fn migrate_at(path: &std::path::Path, captured: &Session) -> Result<Session, String> {
    let mut store = Store::open(path)?;
    let i = store
        .matching(captured)
        .ok_or("Session changed; migration refused")?;
    verified_session(&store.sessions[i])?;
    if store.sessions[i].receipt.is_none() {
        store.sessions[i].receipt = Some(platform::new_receipt()?);
        store.commit()?;
    }
    Ok(store.sessions[i].clone())
}
pub(crate) fn migrate_legacy(captured: &Session) -> Result<Session, String> {
    on_worker(|| migrate_at(&session::store_path(), captured))
}

fn permits_hide(current: &Session, captured: &Session, watcher: bool) -> bool {
    session::same_operation(current, captured)
        && current.receipt.is_some()
        && (current.state == State::Watching || (!watcher && current.state == State::Tracked))
}

trait Visibility {
    type Process;
    fn process(&self, session: &Session) -> Result<Self::Process, String>;
    fn verify(&self, process: &Self::Process) -> Result<(), String>;
    fn windows(&self, pid: u32) -> Result<Vec<isize>, String>;
    fn visible(&self, hwnd: isize) -> bool;
    fn set(&self, process: &Self::Process, hwnd: isize, visible: bool) -> Result<(), String>;
    fn owner(&self, pid: u32, hwnd: isize) -> Result<(), String>;
}
struct NativeVisibility;
impl Visibility for NativeVisibility {
    type Process = platform::Process;
    fn process(&self, s: &Session) -> Result<Self::Process, String> {
        verified_session(s)
    }
    fn verify(&self, p: &Self::Process) -> Result<(), String> {
        p.verify()
    }
    fn windows(&self, pid: u32) -> Result<Vec<isize>, String> {
        platform::checked_windows_of(pid)
    }
    fn visible(&self, hwnd: isize) -> bool {
        platform::is_window_visible(hwnd)
    }
    fn set(&self, p: &Self::Process, hwnd: isize, visible: bool) -> Result<(), String> {
        platform::set_window_visible(p, hwnd, visible)
    }
    fn owner(&self, pid: u32, hwnd: isize) -> Result<(), String> {
        platform::verify_window_owner(pid, platform::window_pid(hwnd))
    }
}

fn hide_at(
    path: &std::path::Path,
    captured: &Session,
    watcher: bool,
    native: &impl Visibility,
) -> Result<Option<usize>, String> {
    let mut store = Store::open(path)?;
    let Some(i) = store.matching(captured) else {
        return Ok(None);
    };
    if !permits_hide(&store.sessions[i], captured, watcher) {
        return Ok(None);
    }
    let process = native.process(&store.sessions[i])?;
    let mut hidden = 0;
    for hwnd in native.windows(captured.pid)? {
        if native.visible(hwnd) {
            native.set(&process, hwnd, false)?;
            hidden += 1;
        }
    }
    if hidden > 0 {
        store.sessions[i].windows_hidden = store.sessions[i]
            .windows_hidden
            .saturating_add(hidden as u32);
        store.commit()?;
    }
    Ok(Some(hidden))
}

pub(crate) fn hide_step(captured: &Session, watcher: bool) -> Result<Option<usize>, String> {
    on_worker(|| hide_at(&session::store_path(), captured, watcher, &NativeVisibility))
}

fn finish_at(path: &std::path::Path, captured: &Session) -> Result<(), String> {
    let mut store = Store::open(path)?;
    if let Some(i) = store.matching(captured) {
        if permits_hide(&store.sessions[i], captured, true) {
            store.sessions[i].state = State::Tracked;
            store.commit()?;
        }
    }
    Ok(())
}
pub(crate) fn finish_watch(captured: &Session) -> Result<(), String> {
    on_worker(|| finish_at(&session::store_path(), captured))
}

pub(crate) fn watch_hide(captured: &Session, watch_secs: u64) -> Result<usize, String> {
    let mut hidden = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(watch_secs);
    loop {
        match hide_step(captured, true)? {
            Some(count) => hidden += count,
            None => return Ok(hidden), // Permanent cancellation, including a removed record.
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    finish_watch(captured)?;
    Ok(hidden)
}

fn restore_at(
    path: &std::path::Path,
    captured: &Session,
    selection: Option<isize>,
    native: &impl Visibility,
) -> Result<usize, String> {
    let mut store = Store::open(path)?;
    let i = store
        .matching(captured)
        .ok_or("Session changed; restore refused")?;
    store.sessions[i].state = State::Restoring;
    store.commit()?; // Durable cancellation precedes queries and visibility changes.
    let process = native.process(&store.sessions[i])?;
    let windows = native.windows(captured.pid)?;
    if let Some(hwnd) = selection {
        if !windows.contains(&hwnd) {
            return Err("Selected window no longer belongs to this session".into());
        }
    }
    let mut shown = 0;
    for hwnd in windows {
        if selection.is_none() || selection == Some(hwnd) {
            native.set(&process, hwnd, true)?;
            shown += 1;
        }
    }
    native.verify(&process)?;
    let remaining = native.windows(captured.pid)?;
    for hwnd in &remaining {
        native.owner(captured.pid, *hwnd)?;
    }
    if remaining.iter().all(|w| native.visible(*w)) {
        store.sessions.remove(i);
        store.commit()?;
    } else if selection.is_none() {
        return Err("Restore incomplete; recoverable session retained".into());
    }
    Ok(shown)
}

pub(crate) fn restore_tracked(
    captured: &Session,
    selection: Option<isize>,
) -> Result<usize, String> {
    on_worker(|| {
        restore_at(
            &session::store_path(),
            captured,
            selection,
            &NativeVisibility,
        )
    })
}

pub(crate) fn show_all(s: &Session) -> Result<usize, String> {
    restore_tracked(s, None)
}

pub(crate) fn launch_tracked(program: &str, args: &[String]) -> Result<Session, String> {
    let process = platform::launch_hidden(program, args)?;
    begin_tracking(&process, program, args)
}

static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) fn stop_background_tracking() {
    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub(crate) fn auto_track(
    program: &str,
    args: &[String],
    file_name: &str,
) -> Result<Option<Session>, String> {
    on_worker(|| {
        let mut store = Store::open(&session::store_path())?;
        if SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst)
            || store.sessions.iter().any(|s| s.program == program)
        {
            return Ok(None);
        }
        let process = match platform::find_process_by_name(file_name) {
            Some((pid, _)) => {
                if let Some(old) = store.sessions.iter().find(|s| s.pid == pid) {
                    verified_session(old)?;
                    return Ok(None);
                }
                platform::Process::open(pid)?
            }
            None => platform::launch_hidden(program, args)?,
        };
        let captured = registration(&process, program, args).map_err(|e| {
            format!(
                "Incomplete autostart: owned child pid {}, creation {}; {e}. No watcher started.",
                process.pid, process.creation_filetime
            )
        })?;
        store.sessions.push(captured.clone());
        store.commit().map_err(|e| {
            format!(
                "Incomplete autostart: owned child pid {}, creation {}; {e}. No watcher started.",
                process.pid, process.creation_filetime
            )
        })?;
        Ok(Some(captured))
    })
}

/// Recovery checks the predecessor and launches/registers its successor under one gate.
pub(crate) fn recover(captured: &Session) -> Result<Option<Session>, String> {
    on_worker(|| {
        let mut store = Store::open(&session::store_path())?;
        let i = store.matching(captured).ok_or("Recovery session changed")?;
        if store.sessions[i].state == State::Restoring {
            return Ok(None);
        }
        if platform::session_process(captured.pid, captured.creation_filetime)?.is_some() {
            return Ok(None);
        }
        if !std::path::Path::new(&captured.program).is_file() {
            return Ok(None);
        }
        let process = platform::launch_hidden(&captured.program, &captured.args)?;
        let successor = registration(&process, &captured.program, &captured.args).map_err(|e| {
            format!(
                "Incomplete recovery: owned child pid {}, creation {}; {e}",
                process.pid, process.creation_filetime
            )
        })?;
        store.sessions[i] = successor.clone();
        store.commit().map_err(|e| {
            format!(
                "Incomplete recovery: owned child pid {}, creation {}; {e}. No watcher started.",
                process.pid, process.creation_filetime
            )
        })?;
        Ok(Some(successor))
    })
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
            let captured = launch_tracked(&program, &args)?;
            let pid = captured.pid;
            let hidden = watch_hide(&captured, watch_secs)?;
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
            let mut lines = Vec::new();
            for (pid, exe) in matches {
                let process = process_for_hide(pid)?;
                let captured = begin_tracking(&process, &exe, &[])?;
                match watch_hide(&captured, 5) {
                    Ok(n) => lines.push(format!("  {exe} (pid {pid}) — {n} window(s) hidden")),
                    Err(e) => lines.push(format!("  {exe} (pid {pid}): {e}")),
                }
            }
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
            let process = process_for_hide(pid)?;
            let captured = begin_tracking(&process, &label, &[])?;
            let hidden = watch_hide(&captured, 5)?;
            Ok(format!(
                "{label} is hidden (pid {pid}, {hidden} window(s) hidden).\nBring it back with: hushrun restore {pid}"
            ))
        }
        Command::List { json } => {
            let sessions = session::load_strict(&path)?;
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
                    format_args!(
                        "{}{}",
                        age_text(s.started_unix, now),
                        verified_session(s)
                            .err()
                            .map(|e| format!(" — {e}"))
                            .unwrap_or_default()
                    )
                ));
            }
            Ok(out.trim_end().to_string())
        }
        Command::Restore { target } => {
            let sessions = session::load_strict(&path)?;
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
                match show_all(s) {
                    Ok(shown) => {
                        lines.push(format!(
                            "{} (pid {}) is back — {shown} window(s) shown.",
                            s.program, s.pid
                        ));
                    }
                    Err(e) => lines.push(format!("{} (pid {}): {e}", s.program, s.pid)),
                }
            }
            Ok(lines.join("\n"))
        }
        Command::RestoreAll => {
            let sessions = session::load_strict(&path)?;
            let mut kept = Vec::new();
            let mut errors = Vec::new();
            for s in &sessions {
                if let Err(e) = show_all(s) {
                    kept.push(s.clone());
                    errors.push(format!("pid {}: {e}", s.pid));
                }
            }
            Ok(format!(
                "Brought back {} hidden program(s).\n{}",
                sessions.len() - kept.len(),
                errors.join("\n")
            )
            .trim_end()
            .into())
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
            if let Ok(_process) = verified_session(s) {
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
                    "    pid {:<7} {} — {}\n",
                    s.pid,
                    s.program,
                    verified_session(s).err().unwrap_or_default()
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
    fn legacy_session_refuses_before_opening_a_pid() {
        let session = Session {
            pid: std::process::id(),
            receipt: None,
            state: State::Tracked,
            creation_filetime: None,
            program: "fixture.exe".into(),
            args: Vec::new(),
            started_unix: 0,
            windows_hidden: 1,
        };
        assert!(verified_session(&session)
            .err()
            .unwrap()
            .contains("Unverified"));
    }

    #[test]
    fn ages() {
        assert_eq!(age_text(100, 130), "30s ago");
        assert_eq!(age_text(100, 100 + 600), "10m ago");
        assert_eq!(age_text(100, 100 + 7200), "2h ago");
        assert_eq!(age_text(100, 100 + 172_800), "2d ago");
        assert_eq!(age_text(100, 50), "0s ago");
    }
}

#[cfg(all(test, windows))]
mod coordination_tests {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};

    fn fixture() -> (std::path::PathBuf, Session) {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("hushrun-watch-b2-resume-tests")
            .join(platform::new_receipt().unwrap());
        let path = directory.join("sessions.json");
        let captured = Session {
            pid: 123,
            creation_filetime: Some(456),
            receipt: Some(platform::new_receipt().unwrap()),
            state: State::Watching,
            program: "inert.exe".into(),
            args: vec![],
            started_unix: 0,
            windows_hidden: 0,
        };
        let mut store = Store::open(&path).unwrap();
        store.sessions.push(captured.clone());
        store.commit().unwrap();
        (path, captured)
    }
    type Pause = Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>;
    struct Mock {
        path: std::path::PathBuf,
        visible: Mutex<Vec<bool>>,
        actions: Mutex<Vec<bool>>,
        deny: bool,
        fail_show: bool,
        fail_query: bool,
        pause: Pause,
    }
    impl Mock {
        fn new(path: &std::path::Path) -> Self {
            Self {
                path: path.into(),
                visible: Mutex::new(vec![true, true]),
                actions: Mutex::new(vec![]),
                deny: false,
                fail_show: false,
                fail_query: false,
                pause: Mutex::new(None),
            }
        }
    }
    impl Visibility for Mock {
        type Process = ();
        fn process(&self, s: &Session) -> Result<(), String> {
            if self.deny {
                return Err("query denied".into());
            }
            platform::verify_identity(s.creation_filetime, 456, true)
        }
        fn verify(&self, _: &()) -> Result<(), String> {
            Ok(())
        }
        fn windows(&self, _: u32) -> Result<Vec<isize>, String> {
            if self.fail_query {
                Err("enumeration denied".into())
            } else {
                Ok(vec![0, 1])
            }
        }
        fn visible(&self, hwnd: isize) -> bool {
            self.visible.lock().unwrap()[hwnd as usize]
        }
        fn set(&self, _: &(), hwnd: isize, visible: bool) -> Result<(), String> {
            let current = session::load_strict(&self.path).unwrap().remove(0);
            assert_eq!(
                current.state,
                if visible {
                    State::Restoring
                } else {
                    State::Watching
                }
            );
            if let Some((entered, release)) = self.pause.lock().unwrap().take() {
                entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            if visible && self.fail_show && hwnd == 1 {
                return Err("show denied".into());
            }
            self.visible.lock().unwrap()[hwnd as usize] = visible;
            self.actions.lock().unwrap().push(visible);
            Ok(())
        }
        fn owner(&self, _: u32, _: isize) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn paused_hide_then_restore_cannot_be_rehidden() {
        let (path, captured) = fixture();
        let (entered, wait_entered) = mpsc::channel();
        let (release, wait_release) = mpsc::channel();
        let mock = Arc::new(Mock::new(&path));
        *mock.pause.lock().unwrap() = Some((entered, wait_release));
        std::thread::scope(|scope| {
            let hide = scope.spawn(|| hide_at(&path, &captured, true, mock.as_ref()));
            wait_entered.recv_timeout(Duration::from_secs(3)).unwrap();
            let (started, wait_started) = mpsc::channel();
            let (restore_path, restore_captured, restore_mock) = (&path, &captured, mock.as_ref());
            let restore = scope.spawn(move || {
                started.send(()).unwrap();
                restore_at(restore_path, restore_captured, None, restore_mock)
            });
            wait_started.recv_timeout(Duration::from_secs(3)).unwrap();
            release.send(()).unwrap();
            assert_eq!(hide.join().unwrap().unwrap(), Some(2));
            assert_eq!(restore.join().unwrap().unwrap(), 2);
        });
        assert!(session::load_strict(&path).unwrap().is_empty());
        assert_eq!(
            hide_at(&path, &captured, true, mock.as_ref()).unwrap(),
            None
        );
        finish_at(&path, &captured).unwrap();
        assert_eq!(
            *mock.actions.lock().unwrap(),
            vec![false, false, true, true]
        );
        assert!(session::load_strict(&path).unwrap().is_empty());
    }

    #[test]
    fn restore_first_and_stale_receipt_stop_late_watchers() {
        let (path, captured) = fixture();
        let mock = Mock::new(&path);
        restore_at(&path, &captured, None, &mock).unwrap();
        assert_eq!(hide_at(&path, &captured, true, &mock).unwrap(), None);
        let mut successor = captured.clone();
        successor.receipt = Some(platform::new_receipt().unwrap());
        let mut store = Store::open(&path).unwrap();
        store.sessions.push(successor.clone());
        store.commit().unwrap();
        drop(store);
        finish_at(&path, &captured).unwrap();
        assert_eq!(hide_at(&path, &captured, false, &mock).unwrap(), None);
        assert_eq!(session::load_strict(&path).unwrap(), vec![successor]);
    }

    #[test]
    fn partial_restore_and_query_failures_retain_restoring_sessions() {
        for mode in 0..4 {
            let (path, captured) = fixture();
            let mut mock = Mock::new(&path);
            *mock.visible.lock().unwrap() = vec![false, false];
            mock.fail_show = mode == 0;
            mock.deny = mode == 1;
            mock.fail_query = mode == 2;
            let result = restore_at(&path, &captured, (mode == 3).then_some(0), &mock);
            assert_eq!(result.is_err(), mode != 3);
            assert_eq!(
                session::load_strict(&path).unwrap()[0].state,
                State::Restoring
            );
            assert_eq!(hide_at(&path, &captured, false, &mock).unwrap(), None);
            finish_at(&path, &captured).unwrap();
            assert_eq!(
                session::load_strict(&path).unwrap()[0].state,
                State::Restoring
            );
        }
    }

    #[test]
    fn corrupt_read_or_registration_failure_never_allows_hide() {
        let (path, captured) = fixture();
        let mock = Mock::new(&path);
        std::fs::write(&path, "broken").unwrap();
        assert!(hide_at(&path, &captured, true, &mock).is_err());
        assert!(restore_at(&path, &captured, None, &mock).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(hide_at(&path, &captured, true, &mock).unwrap(), None);
        assert!(mock.actions.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_restoring_commit_never_changes_visibility() {
        let (path, captured) = fixture();
        let mock = Mock::new(&path);
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
        assert!(restore_at(&path, &captured, None, &mock).is_err());
        assert!(mock.actions.lock().unwrap().is_empty());
        assert_eq!(session::load_strict(&path).unwrap(), vec![captured]);
    }

    #[test]
    fn legacy_identity_and_generation_fail_closed() {
        let (path, mut captured) = fixture();
        captured.receipt = None;
        captured.creation_filetime = None;
        let mut store = Store::open(&path).unwrap();
        store.sessions = vec![captured.clone()];
        store.commit().unwrap();
        drop(store);
        let mock = Mock::new(&path);
        assert_eq!(hide_at(&path, &captured, false, &mock).unwrap(), None);
        assert!(restore_at(&path, &captured, None, &mock).is_err());
        assert!(mock.actions.lock().unwrap().is_empty());
        assert_eq!(
            session::load_strict(&path).unwrap()[0].creation_filetime,
            None
        );
    }
    #[test]
    fn registration_and_explicit_legacy_migration_persist_before_watching() {
        let (path, captured) = fixture();
        let process = platform::Process::open(std::process::id()).unwrap();
        let registered = begin_at(&path, &process, "inert current test", &[]).unwrap();
        assert_eq!(registered.state, State::Watching);
        assert!(registered.receipt.is_some());
        assert!(session::load_strict(&path).unwrap().contains(&registered));
        finish_at(&path, &registered).unwrap();
        let mut store = Store::open(&path).unwrap();
        let i = store.matching(&registered).unwrap();
        assert_eq!(store.sessions[i].state, State::Tracked);
        store.sessions[i].receipt = None;
        let legacy = store.sessions[i].clone();
        store.commit().unwrap();
        drop(store);
        let migrated = migrate_at(&path, &legacy).unwrap();
        assert_eq!(migrated.state, State::Tracked);
        assert!(migrated.receipt.is_some());
        assert!(migrate_at(&path, &legacy).is_err());
        let mut store = Store::open(&path).unwrap();
        store.sessions.clear();
        store.commit().unwrap();
        drop(store);
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
        assert!(begin_at(&path, &process, "inert current test", &[]).is_err());
        assert!(session::load_strict(&path).unwrap().is_empty());
        let mock = Mock::new(&path);
        assert_eq!(hide_at(&path, &captured, true, &mock).unwrap(), None);
        assert!(mock.actions.lock().unwrap().is_empty());
    }

    #[test]
    fn autostart_pid_collision_preserves_retained_record() {
        let retained = Session {
            pid: 123,
            creation_filetime: Some(456),
            receipt: Some(platform::new_receipt().unwrap()),
            state: State::Tracked,
            program: "inert.exe".into(),
            args: vec![],
            started_unix: 0,
            windows_hidden: 0,
        };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("hushrun-collision-b3-tests")
            .join(platform::new_receipt().unwrap())
            .join("sessions.json");
        let mut store = Store::open(&path).unwrap();
        store.sessions.push(retained.clone());
        store.commit().unwrap();
        let before = std::fs::read(&path).unwrap();
        let sentinel = path.with_extension("json.tmp");
        std::fs::write(&sentinel, "untouched").unwrap();
        let mut child = retained.clone();
        child.program = "different-autostart.exe".into();
        child.creation_filetime = Some(789);
        child.receipt = Some(platform::new_receipt().unwrap());
        store.sessions.push(child);
        assert!(store
            .commit()
            .unwrap_err()
            .contains("Invalid session candidate"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "untouched");
        assert_eq!(session::load_strict(&path).unwrap(), vec![retained]);
    }

    #[test]
    fn recovery_pid_collision_preserves_predecessor_and_other_record() {
        let mut predecessor = Session {
            pid: 123,
            creation_filetime: Some(456),
            receipt: Some(platform::new_receipt().unwrap()),
            state: State::Tracked,
            program: "inert.exe".into(),
            args: vec![],
            started_unix: 0,
            windows_hidden: 0,
        };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("hushrun-collision-b3-tests")
            .join(platform::new_receipt().unwrap())
            .join("sessions.json");
        predecessor.state = State::Tracked;
        let mut other = predecessor.clone();
        other.pid += 1;
        other.receipt = Some(platform::new_receipt().unwrap());
        let mut store = Store::open(&path).unwrap();
        store.sessions = vec![predecessor.clone(), other.clone()];
        store.commit().unwrap();
        let before = std::fs::read(&path).unwrap();
        let mut successor = predecessor.clone();
        successor.pid = other.pid;
        successor.creation_filetime = Some(789);
        successor.receipt = Some(platform::new_receipt().unwrap());
        successor.state = State::Watching;
        store.sessions[0] = successor;
        assert!(store.commit().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("json.tmp").exists());
        store.sessions = vec![predecessor.clone(), other.clone()];
        store.sessions[0].receipt = Some("invalid identity".into());
        assert!(store.commit().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(
            session::load_strict(&path).unwrap(),
            vec![predecessor, other]
        );
    }

    #[test]
    fn lock_timeout_changes_neither_tracking_nor_visibility() {
        let (path, captured) = fixture();
        let mock = Mock::new(&path);
        let gate = platform::SessionGate::acquire(path.parent().unwrap()).unwrap();
        assert!(hide_at(&path, &captured, true, &mock)
            .unwrap_err()
            .contains("timeout"));
        assert!(restore_at(&path, &captured, None, &mock)
            .unwrap_err()
            .contains("timeout"));
        assert!(mock.actions.lock().unwrap().is_empty());
        assert_eq!(session::load_strict(&path).unwrap(), vec![captured]);
        drop(gate);
    }
    fn protocol_child(
        path: &std::path::Path,
        captured: &Session,
        mode: &str,
    ) -> std::process::Command {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "app::coordination_tests::visibility_protocol_child",
                "--nocapture",
            ])
            .env("HUSHRUN_TEST_VISIBILITY_PATH", path)
            .env(
                "HUSHRUN_TEST_VISIBILITY_SESSION",
                session::render_store(std::slice::from_ref(captured)),
            )
            .env("HUSHRUN_TEST_VISIBILITY_MODE", mode);
        child
    }

    #[test]
    fn visibility_protocol_child() {
        let Some(path) = std::env::var_os("HUSHRUN_TEST_VISIBILITY_PATH") else {
            return;
        };
        let path = std::path::PathBuf::from(path);
        let captured =
            session::parse_store(&std::env::var("HUSHRUN_TEST_VISIBILITY_SESSION").unwrap())
                .unwrap()
                .remove(0);
        let mock = Mock::new(&path);
        std::fs::write(path.with_extension("ready"), "ready").unwrap();
        match std::env::var("HUSHRUN_TEST_VISIBILITY_MODE")
            .unwrap()
            .as_str()
        {
            "restore" => {
                assert_eq!(restore_at(&path, &captured, None, &mock).unwrap(), 2);
                assert_eq!(*mock.actions.lock().unwrap(), vec![true, true]);
            }
            "late-hide" => {
                assert_eq!(hide_at(&path, &captured, true, &mock).unwrap(), None);
                finish_at(&path, &captured).unwrap();
                assert!(mock.actions.lock().unwrap().is_empty());
            }
            _ => panic!("unexpected visibility protocol mode"),
        }
    }

    #[test]
    fn cross_process_restore_waits_for_hide_and_late_watcher_stays_stopped() {
        let (path, captured) = fixture();
        let (entered, wait_entered) = mpsc::channel();
        let (release, wait_release) = mpsc::channel();
        let mock = Mock::new(&path);
        *mock.pause.lock().unwrap() = Some((entered, wait_release));
        std::thread::scope(|scope| {
            let hide = scope.spawn(|| hide_at(&path, &captured, true, &mock));
            wait_entered.recv_timeout(Duration::from_secs(3)).unwrap();
            let mut restore = protocol_child(&path, &captured, "restore").spawn().unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !path.with_extension("ready").exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "restore child did not reach gate"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(restore.try_wait().unwrap().is_none());
            release.send(()).unwrap();
            assert_eq!(hide.join().unwrap().unwrap(), Some(2));
            assert!(restore.wait().unwrap().success());
        });
        assert!(session::load_strict(&path).unwrap().is_empty());
        assert_eq!(*mock.actions.lock().unwrap(), vec![false, false]);
        assert!(protocol_child(&path, &captured, "late-hide")
            .status()
            .unwrap()
            .success());
        assert!(session::load_strict(&path).unwrap().is_empty());
    }
}
