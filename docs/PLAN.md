# HushRun — Architecture & UX Plan

**Product:** a launcher that runs programs silently in the
background, for Windows. It starts a
program with its windows hidden (nothing on screen, nothing on the
taskbar), keeps hiding windows the program opens later, and can
bring everything back on demand. For the long-running helpers,
updaters, sync tools and game launchers you want working — not
sitting on your taskbar.

**Name:** HushRun (working name; rename freely — nothing depends on
it yet).

## Core workflows (v0.1)

1. `hushrun run <program> [args...]` — launch hidden; watch the
   process for a bounded window (default 15 s, `--watch <seconds>`)
   and hide every window it opens; record a session.
2. `hushrun hide <pid|name>` — hide the windows of a program that
   is already running; record a session.
3. `hushrun list` — sessions HushRun knows about, with live/dead
   status.
4. `hushrun restore <pid|name>` — show the program's windows again
   and drop the session.
5. `hushrun restore-all` — bring everything back (the panic button).

## Design

- **Rust, zero dependencies.** Direct Win32 FFI declarations
  (CreateProcessW, EnumWindows, ShowWindow, OpenProcess) — the same
  house pattern as the WinDevKit DLLs. Safe Rust except the FFI
  module, which is narrow and commented.
- **Platform seam:** all OS interaction sits behind `platform`
  functions (`launch_hidden`, `windows_of`, `set_window_visible`,
  `process_alive`). The Windows implementation is real; a stub for
  other OSes keeps the crate testable anywhere (the logic — CLI,
  session store, matching — is platform-free and unit-tested).
- **Sessions** live in `%LOCALAPPDATA%\HushRun\sessions.json`
  (open format, atomic write: temp file + replace). A session is
  `{ pid, program, args, started_unix, windows_hidden }`.
- **Hiding is honest:** SW_HIDE removes a window from screen and
  taskbar. Some programs (elevated, or with custom renderers) may
  refuse or re-show; the watcher re-hides during its window and
  `list` never claims a window is hidden that isn't recorded.
- **No persistence tricks, no hooks, no injection.** HushRun never
  touches the target process's memory; it only asks the window
  manager to hide/show top-level windows it owns.

## Landed in v0.2 (2026-10-24)

- Resident tray mode (`hushrun tray`, or no arguments): message-
  only window + Shell_NotifyIcon, popup menu (restore one / all,
  run hidden via a GetOpenFileNameW picker, exit restores all),
  a 1-second watcher timer that re-hides windows hidden programs
  open later, prunes dead sessions and refreshes the tooltip,
  a runtime-drawn icon (black square, red border, white "H"),
  a single-instance mutex, and FreeConsole so no console window
  lingers after launch. One binary kept — no separate
  GUI-subsystem build; the CLI remains the scripting interface.
  FFI struct layouts are pinned by compile-time size assertions.

## Not yet (planned, in order)

- Auto-start rules ("always run X hidden at logon") via a config
  file + Startup folder shortcut — no service, no registry Run key
  by default.
- Per-program profiles (watch duration, restore hotkey).

## Verification status

- Logic (CLI parsing, session store round-trip, name matching,
  state-file corruption handling): unit-tested, run in CI.
- Windows behavior (CreateProcess flags, EnumWindows ownership,
  SW_HIDE on real apps): **Unverified** until run on SEAN's Windows
  machine — a capture/verify list lives in docs/ON_MACHINE.md once
  the tray mode lands; for v0.1 the check is: run Notepad hidden,
  confirm no window/taskbar button, `list`, `restore`, Notepad
  returns.
