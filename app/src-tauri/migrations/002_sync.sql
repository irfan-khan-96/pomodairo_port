-- Sync between devices: global ids, an append-only event log, per-field clocks
-- and tombstones. The `task` and `pomodoro_log` tables stay the materialised view
-- the app reads; they are now derived from the event log.

-- Global ids (32 hex chars) alongside the local integer ids the UI uses.
ALTER TABLE task ADD COLUMN uid TEXT;
ALTER TABLE task ADD COLUMN parent_uid TEXT;
UPDATE task SET uid = lower(hex(randomblob(16)));
UPDATE task SET parent_uid = (SELECT p.uid FROM task p WHERE p.id = task.parent_id);
CREATE UNIQUE INDEX task_uid_idx ON task (uid);
CREATE INDEX task_parent_uid_idx ON task (parent_uid);

ALTER TABLE pomodoro_log ADD COLUMN uid TEXT;
ALTER TABLE pomodoro_log ADD COLUMN task_uid TEXT;
UPDATE pomodoro_log SET uid = lower(hex(randomblob(16)));
UPDATE pomodoro_log SET task_uid = (SELECT t.uid FROM task t WHERE t.id = pomodoro_log.task_id);
CREATE UNIQUE INDEX pomodoro_log_uid_idx ON pomodoro_log (uid);
CREATE INDEX pomodoro_log_task_uid_idx ON pomodoro_log (task_uid);

-- Every change, from this device and from others. (device, seq) identifies an
-- event; `hlc` is its hybrid logical clock timestamp (sortable text).
CREATE TABLE sync_event (
  device TEXT    NOT NULL,
  seq    INTEGER NOT NULL,
  hlc    TEXT    NOT NULL,
  op     TEXT    NOT NULL,  -- JSON
  PRIMARY KEY (device, seq)
);
CREATE INDEX sync_event_hlc_idx ON sync_event (hlc);

-- Latest value and clock of each editable task field (last writer wins).
CREATE TABLE task_clock (
  uid   TEXT NOT NULL,
  field TEXT NOT NULL,
  hlc   TEXT NOT NULL,
  value TEXT NOT NULL,      -- JSON
  PRIMARY KEY (uid, field)
);

-- Deleted tasks. Deletion is final; kind/parent keep disturbance counts intact.
CREATE TABLE tombstone (
  uid        TEXT PRIMARY KEY,
  hlc        TEXT NOT NULL,
  kind       TEXT,
  parent_uid TEXT
);

-- This device's random id, and whether pre-sync data has been turned into events.
CREATE TABLE sync_state (
  device       TEXT    NOT NULL,
  bootstrapped INTEGER NOT NULL DEFAULT 0
);
INSERT INTO sync_state (device) VALUES (lower(hex(randomblob(16))));
