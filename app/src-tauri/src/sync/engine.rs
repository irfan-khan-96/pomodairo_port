//! The sync engine: every change to tasks and the pomodoro log is an event in
//! `sync_event`, and the `task` / `pomodoro_log` tables are materialised from
//! those events. Applying the same set of events in any order, any number of
//! times, gives the same result on every device:
//!
//! - editable fields (name, estimate, done, visible, order): newest edit wins,
//!   by hybrid logical clock, tracked per field in `task_clock`;
//! - deletion is final (a tombstone outranks every edit);
//! - pomodoro, unplanned and interruption counts are derived from logged items,
//!   so work done on several devices adds up instead of overwriting.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::hlc::{Hlc, is_device_id};
use crate::error::{Error, Result};
use crate::tasks::{self, TaskKind};

/// One change, as stored and as sent between devices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub device: String,
    pub seq: i64,
    pub hlc: String,
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op {
    CreateTask {
        uid: String,
        kind: TaskKind,
        name: String,
        estimated: Option<i64>,
        parent_uid: Option<String>,
        created_at: String,
        ordinal: i64,
    },
    SetName {
        uid: String,
        name: String,
    },
    SetEstimate {
        uid: String,
        estimated: Option<i64>,
    },
    SetDone {
        uid: String,
        done: bool,
        closed_at: Option<String>,
    },
    SetVisible {
        uid: String,
        visible: bool,
    },
    SetOrdinal {
        uid: String,
        ordinal: i64,
    },
    DeleteTask {
        uid: String,
    },
    LogPomodoro {
        uid: String,
        task_uid: Option<String>,
        started_at: String,
        ended_at: String,
    },
}

/// A new random global id (32 lowercase hex characters).
pub fn new_uid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Run `f` in a transaction, or inside the caller's if one is already open.
pub fn in_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    if conn.is_autocommit() {
        let tx = conn.unchecked_transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    } else {
        f(conn)
    }
}

pub fn device_id(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT device FROM sync_state", [], |r| r.get(0))?)
}

/// Record a change made on this device and apply it.
pub fn record(conn: &Connection, op: Op, now: DateTime<Utc>) -> Result<Event> {
    in_tx(conn, |conn| {
        let device = device_id(conn)?;
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM sync_event WHERE device = ?1",
            [&device],
            |r| r.get(0),
        )?;
        let last: Option<String> =
            conn.query_row("SELECT MAX(hlc) FROM sync_event", [], |r| r.get(0))?;
        let last = last.as_deref().and_then(Hlc::parse);
        let now_ms = u64::try_from(now.timestamp_millis()).unwrap_or(0);
        let hlc = Hlc::tick(last.as_ref(), now_ms, &device).to_string();
        let event = Event {
            device,
            seq,
            hlc,
            op,
        };
        apply(conn, &event)?;
        Ok(event)
    })
}

/// Store and materialise `event` unless it is already known. Returns whether it was new.
pub fn apply(conn: &Connection, event: &Event) -> Result<bool> {
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO sync_event (device, seq, hlc, op) VALUES (?1, ?2, ?3, ?4)",
        params![
            event.device,
            event.seq,
            event.hlc,
            serde_json::to_string(&event.op)?
        ],
    )?;
    if inserted == 0 {
        return Ok(false);
    }
    materialise(conn, &event.hlc, &event.op)?;
    Ok(true)
}

fn tombstoned(conn: &Connection, uid: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM tombstone WHERE uid = ?1", [uid], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Store `value` for `uid.field` if `hlc` is newer than what is stored.
fn set_field(conn: &Connection, uid: &str, field: &str, hlc: &str, value: Value) -> Result<()> {
    conn.execute(
        "INSERT INTO task_clock (uid, field, hlc, value) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (uid, field) DO UPDATE SET hlc = excluded.hlc, value = excluded.value
         WHERE excluded.hlc > task_clock.hlc",
        params![uid, field, hlc, value.to_string()],
    )?;
    Ok(())
}

/// Copy the winning field values from `task_clock` into the task row.
fn refresh_row(conn: &Connection, uid: &str) -> Result<()> {
    let mut stmt = conn.prepare("SELECT field, value FROM task_clock WHERE uid = ?1")?;
    let fields: Vec<(String, String)> = stmt
        .query_map([uid], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (field, value) in fields {
        let v: Value = serde_json::from_str(&value)?;
        match field.as_str() {
            "name" => conn.execute(
                "UPDATE task SET name = ?1 WHERE uid = ?2",
                params![v.as_str(), uid],
            )?,
            "estimated" => conn.execute(
                "UPDATE task SET estimated = ?1 WHERE uid = ?2",
                params![v.as_i64(), uid],
            )?,
            "ordinal" => conn.execute(
                "UPDATE task SET ordinal = ?1 WHERE uid = ?2",
                params![v.as_i64(), uid],
            )?,
            "visible" => conn.execute(
                "UPDATE task SET visible = ?1 WHERE uid = ?2",
                params![v.as_bool(), uid],
            )?,
            "done" => conn.execute(
                "UPDATE task SET done = ?1, closed_at = ?2 WHERE uid = ?3",
                params![v["done"].as_bool(), v["closed_at"].as_str(), uid],
            )?,
            _ => 0,
        };
    }
    Ok(())
}

/// Recompute a task's counts from the items logged against it.
fn recount(conn: &Connection, uid: &str) -> Result<()> {
    conn.execute(
        "UPDATE task SET
           pomodoros = (SELECT COUNT(*) FROM pomodoro_log WHERE task_uid = ?1),
           unplanned = (SELECT COUNT(*) FROM task c WHERE c.parent_uid = ?1 AND c.kind = 'unplanned')
                     + (SELECT COUNT(*) FROM tombstone t WHERE t.parent_uid = ?1 AND t.kind = 'unplanned'),
           interruptions = (SELECT COUNT(*) FROM task c WHERE c.parent_uid = ?1 AND c.kind = 'interruption')
                     + (SELECT COUNT(*) FROM tombstone t WHERE t.parent_uid = ?1 AND t.kind = 'interruption')
         WHERE uid = ?1",
        [uid],
    )?;
    Ok(())
}

fn materialise(conn: &Connection, hlc: &str, op: &Op) -> Result<()> {
    match op {
        Op::CreateTask {
            uid,
            kind,
            name,
            estimated,
            parent_uid,
            created_at,
            ordinal,
        } => {
            if tombstoned(conn, uid)? {
                // Deleted before its creation arrived: keep what the counts need.
                conn.execute(
                    "UPDATE tombstone SET kind = ?2, parent_uid = ?3 WHERE uid = ?1 AND kind IS NULL",
                    params![uid, kind.as_str(), parent_uid],
                )?;
                if let Some(p) = parent_uid {
                    recount(conn, p)?;
                }
                return Ok(());
            }
            conn.execute(
                "INSERT OR IGNORE INTO task (uid, name, kind, parent_uid, parent_id, estimated, ordinal, created_at)
                 VALUES (?1, ?2, ?3, ?4, (SELECT id FROM task WHERE uid = ?4), ?5, ?6, ?7)",
                params![uid, name, kind.as_str(), parent_uid, estimated, ordinal, created_at],
            )?;
            set_field(conn, uid, "name", hlc, json!(name))?;
            set_field(conn, uid, "estimated", hlc, json!(estimated))?;
            set_field(conn, uid, "ordinal", hlc, json!(ordinal))?;
            set_field(
                conn,
                uid,
                "done",
                hlc,
                json!({ "done": false, "closed_at": null }),
            )?;
            set_field(conn, uid, "visible", hlc, json!(true))?;
            refresh_row(conn, uid)?;
            // Link anything that referenced this task before it arrived.
            conn.execute(
                "UPDATE task SET parent_id = (SELECT id FROM task WHERE uid = ?1) WHERE parent_uid = ?1",
                [uid],
            )?;
            conn.execute(
                "UPDATE pomodoro_log SET task_id = (SELECT id FROM task WHERE uid = ?1) WHERE task_uid = ?1",
                [uid],
            )?;
            recount(conn, uid)?;
            if let Some(p) = parent_uid {
                recount(conn, p)?;
            }
        }
        Op::SetName { uid, name } => set_and_refresh(conn, uid, "name", hlc, json!(name))?,
        Op::SetEstimate { uid, estimated } => {
            set_and_refresh(conn, uid, "estimated", hlc, json!(estimated))?
        }
        Op::SetDone {
            uid,
            done,
            closed_at,
        } => set_and_refresh(
            conn,
            uid,
            "done",
            hlc,
            json!({ "done": done, "closed_at": closed_at }),
        )?,
        Op::SetVisible { uid, visible } => {
            set_and_refresh(conn, uid, "visible", hlc, json!(visible))?
        }
        Op::SetOrdinal { uid, ordinal } => {
            set_and_refresh(conn, uid, "ordinal", hlc, json!(ordinal))?
        }
        Op::DeleteTask { uid } => {
            let row: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT kind, parent_uid FROM task WHERE uid = ?1",
                    [uid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (kind, parent_uid) = row.map_or((None, None), |(k, p)| (Some(k), p));
            conn.execute(
                "INSERT OR IGNORE INTO tombstone (uid, hlc, kind, parent_uid) VALUES (?1, ?2, ?3, ?4)",
                params![uid, hlc, kind, parent_uid],
            )?;
            conn.execute("DELETE FROM task WHERE uid = ?1", [uid])?;
            conn.execute("DELETE FROM task_clock WHERE uid = ?1", [uid])?;
            if let Some(p) = parent_uid {
                recount(conn, &p)?;
            }
        }
        Op::LogPomodoro {
            uid,
            task_uid,
            started_at,
            ended_at,
        } => {
            conn.execute(
                "INSERT OR IGNORE INTO pomodoro_log (uid, task_uid, task_id, started_at, ended_at)
                 VALUES (?1, ?2, (SELECT id FROM task WHERE uid = ?2), ?3, ?4)",
                params![uid, task_uid, started_at, ended_at],
            )?;
            if let Some(t) = task_uid {
                recount(conn, t)?;
            }
        }
    }
    Ok(())
}

fn set_and_refresh(
    conn: &Connection,
    uid: &str,
    field: &str,
    hlc: &str,
    value: Value,
) -> Result<()> {
    if tombstoned(conn, uid)? {
        return Ok(());
    }
    set_field(conn, uid, field, hlc, value)?;
    refresh_row(conn, uid)
}

// ---- Exchanging events between devices -------------------------------------------------

/// Per device, the highest `seq` up to which every event is present. Gaps (from
/// an interrupted transfer) stop the count, so missing events are sent again.
#[allow(dead_code)] // used by the LAN sync layer (next phase)
pub fn version_vector(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT device, seq FROM sync_event ORDER BY device, seq")?;
    let mut vv: BTreeMap<String, i64> = BTreeMap::new();
    let mut gapped: Option<String> = None;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (device, seq) = row?;
        if gapped.as_deref() == Some(device.as_str()) {
            continue;
        }
        let have = vv.entry(device.clone()).or_insert(0);
        if seq == *have + 1 {
            *have = seq;
        } else {
            gapped = Some(device);
        }
    }
    vv.retain(|_, seq| *seq > 0);
    Ok(vv)
}

/// Every event this device has that a peer with version vector `theirs` lacks.
#[allow(dead_code)] // used by the LAN sync layer (next phase)
pub fn events_missing(conn: &Connection, theirs: &BTreeMap<String, i64>) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare("SELECT device, seq, hlc, op FROM sync_event ORDER BY hlc")?;
    let mut out = Vec::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })? {
        let (device, seq, hlc, op) = row?;
        if seq > theirs.get(&device).copied().unwrap_or(0) {
            out.push(Event {
                device,
                seq,
                hlc,
                op: serde_json::from_str(&op)?,
            });
        }
    }
    Ok(out)
}

/// Apply events received from another device. Each is validated first, then all
/// are applied in clock order in one transaction. Returns how many were new.
#[allow(dead_code)] // used by the LAN sync layer (next phase)
pub fn ingest(conn: &Connection, events: &[Event]) -> Result<usize> {
    for e in events {
        validate(e)?;
    }
    let mut sorted: Vec<&Event> = events.iter().collect();
    sorted.sort_by(|a, b| a.hlc.cmp(&b.hlc));
    in_tx(conn, |conn| {
        let mut new = 0;
        for e in sorted {
            if apply(conn, e)? {
                new += 1;
            }
        }
        Ok(new)
    })
}

/// Reject malformed events from other devices before they touch the database.
fn validate(e: &Event) -> Result<()> {
    let bad = |what: &str| Err(Error::Invalid(format!("invalid sync event: {what}")));
    if !is_device_id(&e.device) || e.seq <= 0 {
        return bad("device or sequence");
    }
    match Hlc::parse(&e.hlc) {
        Some(h) if h.device == e.device => {}
        _ => return bad("clock"),
    }
    let ts = |s: &str| DateTime::parse_from_rfc3339(s).is_ok();
    let ok = match &e.op {
        Op::CreateTask {
            uid,
            name,
            estimated,
            parent_uid,
            created_at,
            ..
        } => {
            is_device_id(uid)
                && tasks::validate_name(name).is_ok_and(|n| &n == name)
                && tasks::validate_estimate(*estimated).is_ok()
                && parent_uid.as_deref().is_none_or(is_device_id)
                && ts(created_at)
        }
        Op::SetName { uid, name } => {
            is_device_id(uid) && tasks::validate_name(name).is_ok_and(|n| &n == name)
        }
        Op::SetEstimate { uid, estimated } => {
            is_device_id(uid) && tasks::validate_estimate(*estimated).is_ok()
        }
        Op::SetDone { uid, closed_at, .. } => {
            is_device_id(uid) && closed_at.as_deref().is_none_or(ts)
        }
        Op::SetVisible { uid, .. } | Op::SetOrdinal { uid, .. } | Op::DeleteTask { uid } => {
            is_device_id(uid)
        }
        Op::LogPomodoro {
            uid,
            task_uid,
            started_at,
            ended_at,
        } => {
            is_device_id(uid)
                && task_uid.as_deref().is_none_or(is_device_id)
                && ts(started_at)
                && ts(ended_at)
        }
    };
    if ok { Ok(()) } else { bad("content") }
}

// ---- Upgrading data from before sync -------------------------------------------------

/// Turn tasks and pomodoros that existed before sync into events (once), so they
/// reach other devices when this one is paired. Nothing visible changes.
pub fn bootstrap(conn: &Connection) -> Result<()> {
    let done: bool = conn.query_row("SELECT bootstrapped FROM sync_state", [], |r| r.get(0))?;
    if done {
        return Ok(());
    }
    in_tx(conn, |conn| {
        let now = Utc::now();
        let mut stmt = conn.prepare(
            "SELECT uid, kind, name, estimated, parent_uid, created_at, ordinal, done, closed_at, visible
             FROM task ORDER BY id",
        )?;
        let rows: Vec<(Op, bool, Option<String>, bool)> = stmt
            .query_map([], |r| {
                let kind: String = r.get(1)?;
                Ok((
                    Op::CreateTask {
                        uid: r.get(0)?,
                        kind: TaskKind::parse(&kind)?,
                        name: r.get(2)?,
                        estimated: r.get(3)?,
                        parent_uid: r.get(4)?,
                        created_at: r.get(5)?,
                        ordinal: r.get(6)?,
                    },
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for (create, done, closed_at, visible) in rows {
            let Op::CreateTask { uid, .. } = &create else {
                unreachable!()
            };
            let uid = uid.clone();
            record(conn, create, now)?;
            if done {
                record(
                    conn,
                    Op::SetDone {
                        uid: uid.clone(),
                        done,
                        closed_at,
                    },
                    now,
                )?;
            }
            if !visible {
                record(conn, Op::SetVisible { uid, visible }, now)?;
            }
        }
        let mut stmt = conn
            .prepare("SELECT uid, task_uid, started_at, ended_at FROM pomodoro_log ORDER BY id")?;
        let logs: Vec<Op> = stmt
            .query_map([], |r| {
                Ok(Op::LogPomodoro {
                    uid: r.get(0)?,
                    task_uid: r.get(1)?,
                    started_at: r.get(2)?,
                    ended_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for op in logs {
            record(conn, op, now)?;
        }
        conn.execute("UPDATE sync_state SET bootstrapped = 1", [])?;
        Ok(())
    })
}
