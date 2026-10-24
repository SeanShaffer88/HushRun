# HushRun

## TL;DR

HushRun runs programs silently in the background on Windows: it
starts a program with its windows hidden — nothing on screen,
nothing on the taskbar — keeps hiding windows the program opens
later, and brings everything back when you ask. One small
executable, no dependencies, works offline.

## Download

Grab the latest from the
[GitHub Releases](https://github.com/SeanShaffer88/HushRun/releases)
page — an installer (`HushRun-Setup-…-x64.exe`, per-user, no
admin needed) or the portable single-file exe
(`HushRun-…-portable.exe`). Builds are unsigned, so Windows
SmartScreen will ask once — choose "More info → Run anyway".
For portable use, drop a `portable.txt` next to the exe and
HushRun keeps its sessions and settings right there.

## Why

Some programs earn a place in your day but not on your taskbar:
sync tools, updaters, chat clients, game launchers, that one
utility that insists on opening a window at startup. HushRun lets
them work without the clutter — and unlike closing the window,
the program keeps running and can be brought back exactly as it
was.

## Usage

### Tray

```
hushrun            (or: hushrun tray)
```

Starts the tray icon (bright while anything is hidden, dim
when nothing is). Its menu lists the programs currently hidden
— each opens a submenu to restore all of its windows or just
one — and offers **Restore all**, a **Hide a running program**
submenu (every app with a visible window — pick one to hide
it), **Run a program hidden…** (pick an .exe), **Settings…**,
settings export/import, **Restart as administrator**, and
**Exit**, which restores everything before quitting. While the
tray is running it watches hidden programs continuously, so a
window one of them opens later is tucked away too. The tray and
the command line share the same session list.

Two global hotkeys work while the tray runs: **Ctrl+Alt+H**
hides the program in the foreground window, and **Ctrl+Alt+U**
restores everything. (If another app already owns a combo, that
hotkey just doesn't register; the menu is unaffected.)

Small kindnesses built in: you get a quiet balloon when a
program is hidden, restored, or exits while hidden; if the tray
is killed or the PC loses power, the next launch offers to
restore whatever was left hidden; and if Explorer restarts, the
icon puts itself back.

### Command line

```
hushrun run [--watch <seconds>] <program> [args...]
hushrun <program> [args...]          shorthand for run
hushrun hide <pid|name|pattern>      hide a running program's windows
                                     (pattern may contain *, e.g. 'note*')
hushrun list [--json]                list hidden programs
hushrun restore <pid|name|pattern>   bring a hidden program back
hushrun restore-all                  bring everything back
hushrun doctor                       check the store, config and sessions
hushrun export <file>                write settings to a file
hushrun import <file>                load settings from a file
hushrun version
```

### Settings, profiles, and portable mode

The tray's **Settings…** window holds the global preferences
(start with Windows, notifications, default watch window) and
your **program entries**: per-program arguments, watch window
and start delay — and, with "Run hidden at tray start" ticked,
an auto-start rule the tray applies every time it launches.
Programs that were hidden when you shut down are relaunched
hidden at the next tray start. Settings live in
`%LOCALAPPDATA%\HushRun\config.json`; put a `portable.txt`
next to `hushrun.exe` and sessions + settings live beside the
exe instead.

`run` starts the program hidden and watches it for 15 seconds by
default, hiding any windows it opens in that time (many programs
create their window a moment after starting). Sessions are
remembered in `%LOCALAPPDATA%\HushRun\sessions.json`, so
`restore` works from any later invocation — even after HushRun
itself has exited.

## How it works

- The program is started with the window manager told to keep it
  hidden, and without a console window flashing up.
- During the watch window, HushRun enumerates the process's
  top-level windows and hides any that become visible.
- `restore` shows the process's windows again. No hooks, no
  injection, no touching the other program's memory — HushRun only
  ever asks the window manager to hide or show windows.

## Honest limits

- A program running elevated (as administrator) can only be
  hidden by an equally elevated HushRun — that's a Windows rule,
  not a HushRun choice.
- A program that deliberately re-shows its window after the watch
  period will appear; run it again with a longer `--watch`, or
  `hide` it again.
- Hiding is per top-level window: child dialogs a program opens
  later, outside the watch window, are not auto-hidden by the CLI
  alone — the tray mode watches continuously and covers that case
  while it is running.

## Build

```
cargo build --release
cargo test
```

Rust, zero dependencies. Windows 10 and 11, x64 (the code is
architecture-neutral; ARM64 builds are expected to work but are
unverified).

## Status

v0.4.0 — settings window, program profiles and auto-start
rules, relaunch after reboot, per-window restore, icon state,
restart-as-administrator, settings export/import, `doctor`,
`list --json`, wildcard hide/restore, and portable mode, on top
of the 0.3.0 tray and CLI. The 0.1–0.3 behavior was verified on
a real Windows desktop on 2026-10-24; the new 0.4.0 surface is
**partially verified** (unit tests + CI) until the next
on-machine pass. See `docs/PLAN.md`.

## License

MIT — see `LICENSE`.
