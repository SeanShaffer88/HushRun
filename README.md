# HushRun

## TL;DR

HushRun runs programs silently in the background on Windows: it
starts a program with its windows hidden — nothing on screen,
nothing on the taskbar — keeps hiding windows the program opens
later, and brings everything back when you ask. One small
executable, no installer, no dependencies, works offline.

## Why

Some programs earn a place in your day but not on your taskbar:
sync tools, updaters, chat clients, game launchers, that one
utility that insists on opening a window at startup. HushRun lets
them work without the clutter — and unlike closing the window,
the program keeps running and can be brought back exactly as it
was.

## Usage

```
hushrun run [--watch <seconds>] <program> [args...]
hushrun <program> [args...]          shorthand for run
hushrun hide <pid|name>              hide a running program's windows
hushrun list                         list hidden programs
hushrun restore <pid|name>           bring a hidden program back
hushrun restore-all                  bring everything back
```

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
  later, outside the watch window, are not auto-hidden in v0.1.
  The resident tray mode on the roadmap covers that case.

## Build

```
cargo build --release
cargo test
```

Rust, zero dependencies. Windows 10 and 11, x64 (the code is
architecture-neutral; ARM64 builds are expected to work but are
unverified).

## Status

v0.1.0 — the command-line core. Logic is unit-tested in CI;
behavior on a real Windows desktop is **unverified** until the
on-machine check (run Notepad hidden, confirm no window or
taskbar button, restore it) has been run. A resident tray mode is
planned next; see `docs/PLAN.md`.

## License

MIT — see `LICENSE`.
