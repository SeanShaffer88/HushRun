# Changelog

## 0.4.0 — 2026-10-24

The remaining enhancement batch: settings, profiles, automation,
and scripting.

### Added

- **Settings window** (tray → Settings…): start HushRun with
  Windows (per-user Run entry), notification toggle, default
  watch window, and the program-entries editor.
- **Program entries**: per-program profiles (arguments, watch
  window, start delay) and auto-start rules — entries marked
  "Run hidden at tray start" are hidden, or launched hidden,
  whenever the tray starts.
- **Relaunch after reboot**: sessions whose program exited while
  HushRun was away are relaunched hidden at tray start when the
  program file is still there.
- **Icon state**: the tray icon is bright while anything is
  hidden and dims when nothing is.
- **Per-window restore**: each hidden program's tray entry is a
  submenu — restore all its windows or just one (hidden windows
  are checkmarked).
- **Restart as administrator** (tray menu): relaunches the tray
  elevated so it can hide elevated programs; declining the UAC
  prompt leaves the current instance running.
- **About HushRun** (tray menu).
- **Settings export/import**: tray menu items and
  `hushrun export <file>` / `hushrun import <file>`; import keeps
  the previous settings as `config.json.bak`.
- **`hushrun doctor`**: reports the data folder, settings
  health, tray marker, and every session's live state.
- **`hushrun list --json`**: the session list as JSON.
- **Wildcard hide/restore**: `hushrun hide 'note*'` hides every
  running match; restore accepts the same patterns.
- **Portable mode**: a `portable.txt` next to `hushrun.exe`
  keeps sessions and settings beside the exe instead of in
  `%LOCALAPPDATA%`.

### Notes

- Settings live in `config.json` next to `sessions.json`.
  The tray's Settings "Arguments" field splits on spaces (quote
  handling for exotic args remains a CLI job for now).
- Everything from 0.3.0 is unchanged in behavior. New tray
  surface is unverified on a real desktop until the next
  on-machine check.

## 0.3.0 — 2026-10-24

### Added

- **Hide a running program from the tray**: the menu gains a
  "Hide a running program" submenu listing every app with a
  visible window — pick one and it's hidden and recorded, no
  terminal needed.
- **Global hotkeys** (while the tray runs): `Ctrl+Alt+H` hides
  the foreground window's program; `Ctrl+Alt+U` restores
  everything. If another app owns a combo, that hotkey simply
  doesn't register and the menu is unaffected.
- **Unclean-exit recovery**: if the tray is killed or the PC
  loses power, the next launch notices and offers to restore
  the programs that were left hidden.
- **Explorer-restart recovery**: the tray icon re-registers
  itself if Explorer restarts.
- **Balloons**: a quiet notification when a program is hidden,
  restored, started hidden, or exits while hidden.

### Notes

- The CLI is unchanged. Tray behavior on a real desktop remains
  unverified until the on-machine check.

## 0.2.0 — 2026-10-24

### Added

- **Tray mode** (`hushrun tray`, or just `hushrun` with no
  arguments): a notification-area icon with a menu that lists
  hidden programs, restores one or all of them, launches a new
  program hidden via a file picker, and restores everything on
  exit. While the tray runs, it checks once a second and hides
  windows a hidden program opens, so later dialogs are covered.
  The icon is drawn at runtime (black square, red border, white
  "H"); single-instance is enforced; starting the tray detaches
  the console window.

### Notes

- Tray mode is Windows-only, like hiding itself. Everything in
  0.1.0 is unchanged; the CLI stays the scripting interface.

## 0.1.0 — 2026-10-24

- First build: command-line core.
- `run` (plus shorthand) starts a program hidden and watches it
  for a configurable window (default 15 s), hiding windows it
  opens; sessions persist across invocations.
- `hide` hides a running program's windows by pid or .exe name.
- `list` shows hidden sessions with age and prunes exited ones.
- `restore` / `restore-all` bring windows back.
- Zero dependencies; direct Win32 calls; sessions stored as JSON
  in `%LOCALAPPDATA%\HushRun\sessions.json` (atomic writes).
- Status: logic unit-tested; Windows desktop behavior unverified
  pending the on-machine check.
