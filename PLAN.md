# Pomodairo 2 — Implementation Plan

A fast, cross-platform Pomodoro timer and task tracker.

## Status (2026-10-21)

Milestones 1–8 are implemented. Differences from the plan below, decided along the way:

- **Breaks count down** (the original counted up), so the clock and progress bar behave the same in every phase.
- **Settings and statistics open in their own windows**; the main window is too small for the original's in-window panels.
- **No "Next" button**: clicking a row in the task list selects the task, and "Done" moves on to the next open task automatically.
- **The database location** is stored in `database-location.txt` in the app config folder, since it can't live inside the database. Moving it copies the database if the target folder has none, then restarts.
- **`schema_version` table → `PRAGMA user_version`** for migrations.
- **Statistics "Count" view** counts pomodoros from the pomodoro log by the day they were completed. The original counted by the day the task was created.
- **Icon and sounds** are original and generated from source: a "segment tomato" app icon (SVG), a live progress-ring tray icon drawn in Rust, and a synthesised "soft digital" sound set with a distinct break-over chime.
- **Device sync (in progress), no cloud, built in.** Phase 1 done: every change is an event (`sync_event`) with a hybrid logical clock; the task tables are materialised from events. Newest edit wins per field, deletion is final, and pomodoro/unplanned/interruption counts are derived from logged items, so work on several devices adds up. Existing databases are backed up (`pomodairo.db.v1.bak`) and converted on first start. Next: phase 2, LAN sync (discovery, numeric-comparison pairing, TLS 1.3 with pinned certificates, private networks only), then phase 3, the Devices tab and the hand-off indicator.
- **Not yet done:** code signing, auto-update.

## 1. Decisions

| Area | Choice | Why |
|---|---|---|
| Shell | **Tauri 2** | Small installers, fast startup, low RAM, native plugins |
| Backend | **Rust** | Timer engine, database, sound, tray, notifications |
| Frontend | **Svelte 5 + TypeScript + Vite** | Lightweight and reactive; close to Flex's binding model |
| Database | **SQLite via `rusqlite`** (bundled) | Same model as the original; no system dependency |
| Sound | **`rodio`** (Rust) | Plays sound even when the window is hidden or throttled |
| Charts | **Chart.js** | Enough for the statistics views; easy theming |
| Package manager | npm | Already installed |
| Targets | Windows 10+, macOS 11+, Debian 12+ / Ubuntu 22.04+ (`.deb`) | Linux focuses on Debian-based distros for now |

Out of scope for v1: the Twitter integration (dropped), migrating data from the old app (not needed), auto-update and code signing (deferred until public releases).

## 2. Feature parity checklist (v1)

**Timer**
- [ ] Pomodoro, short break and long break, each with a configurable length (defaults 25 / 5 / 15 min, long break every 4)
- [ ] Start / Stop; after time-out: alarm, blinking display, and the break starts automatically after 5 s
- [ ] Ticking sound (optionally fades out) and an alarm sound, each replaceable with a custom file
- [ ] Window title (and taskbar) shows the remaining time and the active task
- [ ] Bring the window to front and flash the taskbar on time-out

**Tasks**
- [ ] Add and edit a task (name plus estimated pomodoros); `#hashtags` in names
- [ ] Task list columns: Name, E(stimated), P(omodoros), U(nplanned), I(nterruptions)
- [ ] Select the active task, Done, Next task, reorder (drag), hide, delete
- [ ] Log Unplanned / Interruption against the active task (each is stored as its own child record)

**Window**
- [ ] Frameless, draggable, custom close/minimise buttons
- [ ] Three sizes: **mini** (clock only), **short** (clock and buttons), **full** (with the task list)
- [ ] Always on top (option), remember position and size (option)

**Statistics window** (separate, normally decorated window)
- [ ] Pomodoros completed per day or date range, with a name filter
- [ ] Interruptions / unplanned items per day or range
- [ ] Pomodoros per day (estimated vs. actual, delta)
- [ ] Weekly "reality factor" (actual ÷ estimated)
- [ ] Hashtag lists for pomodoros and interruptions

**Settings**
- [ ] General: always on top, remember position, update taskbar
- [ ] Sounds: alarm and ticking files, fade ticking, volume
- [ ] Timing: pomodoro, short break and long break lengths, long-break interval
- [ ] Advanced: database location

## 3. Architecture

```
┌──────────────────────── Svelte UI (webview) ────────────────────────┐
│ TimerView │ TaskList │ Dialogs (task / interruption) │ Settings │ Stats │
└──────────────▲─────────────────────────────────────┬────────────────┘
     events    │ timer://tick, timer://state,        │ invoke("cmd", …)
               │ tasks://changed                     ▼
┌──────────────┴──────────────────── Rust core ───────────────────────┐
│ timer.rs   state machine and tick loop (tokio interval, wall-clock) │
│ db.rs      rusqlite connection, migrations, queries                 │
│ tasks.rs   task commands (CRUD, reorder, counters)                  │
│ stats.rs   statistics queries                                       │
│ config.rs  key/value settings, typed accessors                      │
│ audio.rs   rodio: tick loop with fade, alarm                        │
│ window.rs  view-size modes, always-on-top, position persistence     │
│ tray.rs    tray icon and menu (start/stop, show, quit)              │
└─────────────────────────────────────────────────────────────────────┘
```

**Principles**
- **Rust owns the truth.** The timer state lives in Rust and is computed from `Instant`/wall-clock deadlines, never by counting ticks. The UI only renders it, so the timer stays accurate when the window is minimised and webview timers are throttled.
- Commands are typed with `serde`. Rust emits events so the UI can react to changes.
- Every SQL query uses bound parameters; the old code built queries by string concatenation, which allowed SQL injection.

### Timer state machine
```
Idle ──start──▶ Pomodoro ──deadline──▶ TimeOut ──(5 s)──▶ ShortBreak | LongBreak
  ▲                │ stop                                     │ deadline
  └────────────────┴──────────────────── BreakOver ◀──────────┘
```
On a pomodoro's deadline: increment the task's `pomodoros`, play the alarm, emit the event, flash the window, and pick the next break type from the long-break counter.

## 4. Data model (fresh schema, no migration)

```sql
CREATE TABLE task (
  id            INTEGER PRIMARY KEY,
  name          TEXT    NOT NULL,
  kind          TEXT    NOT NULL CHECK (kind IN ('pomodoro','unplanned','interruption')),
  parent_id     INTEGER REFERENCES task(id) ON DELETE CASCADE,
  estimated     INTEGER,
  pomodoros     INTEGER NOT NULL DEFAULT 0,
  unplanned     INTEGER NOT NULL DEFAULT 0,
  interruptions INTEGER NOT NULL DEFAULT 0,
  done          INTEGER NOT NULL DEFAULT 0,
  visible       INTEGER NOT NULL DEFAULT 1,
  ordinal       INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT    NOT NULL,   -- ISO-8601 UTC
  closed_at     TEXT
);
CREATE TABLE pomodoro_log (         -- new: one row per completed pomodoro, for accurate stats
  id INTEGER PRIMARY KEY, task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
  started_at TEXT NOT NULL, ended_at TEXT NOT NULL
);
CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE schema_version (version INTEGER NOT NULL);
```
The database is stored in the OS app-data directory (`app_data_dir()`) unless overridden in Advanced settings. Migrations run on startup.

`pomodoro_log` is a small, deliberate change: the old app could only produce statistics from task creation dates. Logging each pomodoro gives real "done per day" numbers and makes future features (heatmaps, export) easy.

## 5. Design (original look, refined)

Kept from the original:
- Dark charcoal panels (`#292929` and similar), a large yellow seven-segment clock, the same button layout ("Start Timer", "Done", "Task List", "Unplanned", "Interruption"), and the three window sizes.

Refinements:
- **Colour by state:** the clock is yellow during a pomodoro, green during a break, and red and blinking on time-out.
- **Thin progress bar** under the clock (the old code had one hidden behind a flag).
- **Font:** replace *digital-7* (free for personal use only; commercial licence needed) with **DSEG7 Classic** (SIL Open Font License), which looks nearly identical and can be shipped legally.
- **Vector (SVG) icons** so it stays sharp on HiDPI screens; CSS variables hold all colours so themes can be added later.
- **Tray icon** whose tooltip shows the remaining time and task, with a menu: Start/Stop, Show/Hide, Quit.
- **Native OS notification** on time-out and when a break ends, in addition to the sound.
- Keyboard shortcuts in the window: `Space` start/stop, `N` new task, `I` interruption, `U` unplanned, `M` mini view.

## 6. Project layout

```
pomodairo_new/
├─ app/
│  ├─ src/                    # Svelte frontend
│  │  ├─ lib/components/      # Timer, TaskList, TaskRow, Dialog, ...
│  │  ├─ lib/stores/          # timer, tasks, config stores (fed by Rust events)
│  │  ├─ lib/api.ts           # typed invoke wrappers
│  │  ├─ routes/ or windows/  # main.svelte, stats.svelte, settings.svelte
│  │  └─ styles/theme.css
│  ├─ src-tauri/
│  │  ├─ src/{main,lib,timer,db,tasks,stats,config,audio,window,tray}.rs
│  │  ├─ migrations/001_init.sql
│  │  ├─ resources/{alarm,tick}.ogg|wav
│  │  ├─ icons/
│  │  └─ tauri.conf.json
│  └─ package.json
└─ .github/workflows/build.yml  # Windows / macOS / Ubuntu matrix (later)
```

## 7. Milestones

1. **Scaffold.** `create-tauri-app` (Svelte-TS). Frameless 320×167 window, theme CSS, DSEG7 font. App launches on Windows.
2. **Timer engine.** Rust state machine, tick events, start/stop, breaks, alarm and tick sounds through rodio. Unit tests for the state machine using a mock clock.
3. **Database and tasks.** Schema, migrations, task commands, task list UI with E/P/U/I columns, add/edit dialog, done/next/reorder/hide/delete, unplanned and interruption logging.
4. **Window behaviour.** Mini/short/full sizes, drag, always on top, remember position and size, flash/focus on time-out.
5. **Settings.** General, sounds (file picker and preview), timing, advanced (database location).
6. **Statistics window.** All five views plus hashtag lists, date range picker and filter.
7. **OS integration.** Tray icon with countdown tooltip, native notifications, single-instance lock.
8. **Packaging.** `.msi`/`.exe` on Windows, `.dmg` on macOS, `.deb` on Debian; GitHub Actions build matrix.

Deferred to the release phase: auto-updater (`tauri-plugin-updater` with GitHub Releases), code signing (Windows certificate, Apple Developer ID), autostart, global hotkeys, data export.

## 8. Platform notes

- **Windows:** WebView2 is preinstalled on Windows 11; the installer bootstraps it on Windows 10.
- **Debian/Ubuntu build dependencies:** `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev build-essential`. At runtime the `.deb` declares the matching libraries.
- **Wayland:** always-on-top and window positioning are limited by the compositor (GNOME ignores app-set positions). Accept this, and document running with `GDK_BACKEND=x11` as a fallback.
- **macOS:** a frameless window needs `macOSPrivateApi` for transparency, and the tray uses a template icon.

## 9. Local toolchain (checked 2026-10-19)

Rust 1.98.1, Node 24.20, npm 12.2, Git 2.55, Visual Studio 2022 (MSVC build tools) and WebView2 155 are all present, so the scaffold can be built on Windows right away. Linux and macOS builds will run in CI or a VM/WSL later.
