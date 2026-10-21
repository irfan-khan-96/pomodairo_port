# Pomodairo

A small, always-at-hand Pomodoro timer with a task list, interruption tracking and
statistics, for Windows, macOS and Linux (Debian/Ubuntu), built with
[Tauri 2](https://tauri.app), Rust and Svelte.

## Features

- Pomodoro, short-break and long-break timer with ticking and alarm sounds (custom sounds supported)
- Task list with estimates and counts of pomodoros, unplanned items and interruptions (E/P/U/I)
- Mini, short and full views; always-on-top; remembers window position and size
- Statistics: finished tasks, interruptions, pomodoros per day and the weekly "reality factor"
- Tray icon with a live countdown, desktop notifications, single instance
- Data in a local SQLite database whose folder you can move (e.g. into a synced folder)

## Development

Prerequisites: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) LTS, and the
[Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your OS. On
Debian/Ubuntu:

```bash
sudo apt install build-essential libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libxdo-dev
```

Then, from `app/`:

```bash
npm install
npm run tauri dev
```

Useful environment variables (debug builds only):

| Variable | Effect |
|---|---|
| `POMODAIRO_FAST=1` | Every minute of timer length becomes a second, to check a full cycle quickly |
| `POMODAIRO_DB=<path>` | Use a scratch database file instead of your real one |

Checks:

```bash
npm run check
```

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

## Building installers

From `app/`, `npm run tauri build` builds the installers for the current OS into
`src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| Windows | `nsis/Pomodairo_<version>_x64-setup.exe`, `msi/Pomodairo_<version>_x64_en-US.msi` |
| macOS | `macos/Pomodairo.app`, `dmg/Pomodairo_<version>_<arch>.dmg` |
| Debian / Ubuntu | `deb/Pomodairo_<version>_amd64.deb` (Debian 12+, Ubuntu 22.04+) |

GitHub Actions (`.github/workflows/build.yml`) builds all three on every push and pull
request and attaches the installers to the run. To make a release, bump `version` in
`app/src-tauri/tauri.conf.json`, `app/src-tauri/Cargo.toml` and `app/package.json`, then
push a tag such as `v0.2.0`; the workflow creates a draft GitHub release with the
installers attached.

The installers are not code-signed yet, so Windows SmartScreen and macOS Gatekeeper
will warn on first launch.

## Project layout

```
app/
  src/                 Svelte UI (main window, settings, statistics)
  src-tauri/src/       Rust: timer engine, database, audio, tray, window handling
  src-tauri/migrations SQLite schema
PLAN.md                Design decisions and milestones
```

## Credits

- Clock font: [DSEG](https://www.keshikan.net/fonts-e.html) by keshikan, SIL Open Font License 1.1 (`app/static/fonts/DSEG-LICENSE.txt`).
