-- Tasks, unplanned items and interruptions share one table, as in the original app.
-- Unplanned items and interruptions point at the task they interrupted via parent_id.
CREATE TABLE task (
  id            INTEGER PRIMARY KEY,
  name          TEXT    NOT NULL,
  kind          TEXT    NOT NULL CHECK (kind IN ('pomodoro', 'unplanned', 'interruption')),
  parent_id     INTEGER REFERENCES task(id) ON DELETE SET NULL,
  estimated     INTEGER,
  pomodoros     INTEGER NOT NULL DEFAULT 0,
  unplanned     INTEGER NOT NULL DEFAULT 0,
  interruptions INTEGER NOT NULL DEFAULT 0,
  done          INTEGER NOT NULL DEFAULT 0,
  visible       INTEGER NOT NULL DEFAULT 1,
  ordinal       INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT    NOT NULL,  -- RFC 3339, UTC
  closed_at     TEXT
);

CREATE INDEX task_list_idx ON task (visible, kind, ordinal);

-- One row per completed pomodoro, for accurate statistics.
CREATE TABLE pomodoro_log (
  id         INTEGER PRIMARY KEY,
  task_id    INTEGER REFERENCES task(id) ON DELETE SET NULL,
  started_at TEXT NOT NULL,
  ended_at   TEXT NOT NULL
);

CREATE INDEX pomodoro_log_ended_idx ON pomodoro_log (ended_at);

CREATE TABLE config (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
