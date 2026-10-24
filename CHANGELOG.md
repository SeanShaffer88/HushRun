# Changelog

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
