# Pomodairo

A small, always-at-hand Pomodoro timer with a task list, interruption tracking and
statistics, for Windows, macOS and Linux (Debian/Ubuntu), built with
[Tauri 2](https://tauri.app), Rust and Svelte.

## Features

- Pomodoro, short-break and long-break timer with ticking and alarm sounds (custom sounds supported)
- Task list with estimates and counts of pomodoros, unplanned items and interruptions (E/P/U/I)
- Mini, short and full views; always-on-top; remembers window position and size
- Statistics: finished tasks, interruptions, pomodoros per day and the weekly "reality factor"
- Tray icon with a live progress ring and countdown, desktop notifications, single instance
- Sync between your own computers over your home network: no cloud, no account; pair once by comparing a 6-digit code, and a ✓ tells you when it's safe to switch devices
- Data in a local SQLite database whose folder you can move
- Automatic updates on Windows and macOS (signed; asks before installing)

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

## Icons and sounds

Both are generated from source, so they can be tweaked and regenerated:

- **Icons:** edit `src-tauri/icons/source/icon.svg` (48px and up) or `icon-small.svg` (16–32px), then run `npm run icons` from `app/`.
- **Sounds:** edit and run `python scripts/make-sounds.py` from `app/` (standard library only). It writes `tick.wav`, `alarm.wav` and `break-end.wav` to `src-tauri/resources/sounds/`.
- **Tray icon:** the progress ring is drawn at runtime by `src-tauri/src/tray_icon.rs`.

## Building installers

From `app/`, `npm run tauri build` builds the installers for the current OS into
`src-tauri/target/release/bundle/`:

| OS | Output |
|---|---|
| Windows | `nsis/Pomodairo_<version>_x64-setup.exe`, `msi/Pomodairo_<version>_x64_en-US.msi` |
| macOS | `macos/Pomodairo.app`, `dmg/Pomodairo_<version>_<arch>.dmg` |
| Debian / Ubuntu | `deb/Pomodairo_<version>_amd64.deb` (Debian 12+, Ubuntu 22.04+) |

The installers are not code-signed yet, so Windows SmartScreen and macOS Gatekeeper
will warn on first launch. Installed copies on Windows and macOS update themselves
(Settings → General → "Check for updates automatically").

Making a release is described in [RELEASING.md](RELEASING.md).

## Project layout

```
app/
  src/                 Svelte UI (main window, settings, statistics)
  src-tauri/src/       Rust: timer engine, database, audio, tray, window handling
  src-tauri/migrations SQLite schema
  scripts/             Icon and sound generators
PLAN.md                Design decisions and milestones
RELEASING.md           How to make a release
```

## License

MIT. See [LICENSE](LICENSE).

## Credits

- Clock font: [DSEG](https://www.keshikan.net/fonts-e.html) by keshikan, SIL Open Font License 1.1 (`app/static/fonts/DSEG-LICENSE.txt`).
