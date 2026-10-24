# Changelog

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
