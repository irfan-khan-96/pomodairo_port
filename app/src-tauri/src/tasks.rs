//! Task storage: the task list, unplanned items, interruptions and completed pomodoros.
//!
//! Functions take the connection and the current time explicitly so they can be
//! tested against an in-memory database.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::sync::engine::{Op, in_tx, new_uid, record};

pub const MAX_NAME_LEN: usize = 200;
pub const MAX_ESTIMATE: i64 = 99;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Pomodoro,
    Unplanned,
    Interruption,
}

impl TaskKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TaskKind::Pomodoro => "pomodoro",
            TaskKind::Unplanned => "unplanned",
            TaskKind::Interruption => "interruption",
        }
    }

    pub(crate) fn parse(s: &str) -> rusqlite::Result<Self> {
        match s {
            "pomodoro" => Ok(TaskKind::Pomodoro),
            "unplanned" => Ok(TaskKind::Unplanned),
            "interruption" => Ok(TaskKind::Interruption),
            other => Err(rusqlite::Error::InvalidColumnType(
                0,
                format!("unknown task kind {other}"),
                rusqlite::types::Type::Text,
            )),
        }
    }

    /// Pomodoro tasks and unplanned items appear in the task list; interruptions do not.
    fn listed(self) -> bool {
        self != TaskKind::Interruption
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: i64,
    pub name: String,
    pub kind: TaskKind,
    pub parent_id: Option<i64>,
    pub estimated: Option<i64>,
    pub pomodoros: i64,
    pub unplanned: i64,
    pub interruptions: i64,
    pub done: bool,
    pub ordinal: i64,
    pub created_at: String,
    pub closed_at: Option<String>,
}

const COLUMNS: &str = "id, name, kind, parent_id, estimated, pomodoros, unplanned, interruptions, \
                       done, ordinal, created_at, closed_at";

fn from_row(row: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: TaskKind::parse(&row.get::<_, String>(2)?)?,
        parent_id: row.get(3)?,
        estimated: row.get(4)?,
        pomodoros: row.get(5)?,
        unplanned: row.get(6)?,
        interruptions: row.get(7)?,
        done: row.get(8)?,
        ordinal: row.get(9)?,
        created_at: row.get(10)?,
        closed_at: row.get(11)?,
    })
}

pub fn timestamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("name must not be empty".into()));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(Error::Invalid(format!(
            "name must be at most {MAX_NAME_LEN} characters"
        )));
    }
    Ok(name.to_string())
}

pub(crate) fn validate_estimate(estimated: Option<i64>) -> Result<Option<i64>> {
    match estimated {
        Some(e) if !(0..=MAX_ESTIMATE).contains(&e) => Err(Error::Invalid(format!(
            "estimate must be between 0 and {MAX_ESTIMATE}"
        ))),
        e => Ok(e),
    }
}

/// Visible tasks and unplanned items in list order.
pub fn list(conn: &Connection) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM task WHERE visible = 1 AND kind IN ('pomodoro', 'unplanned') \
         ORDER BY ordinal, created_at, uid"
    ))?;
    let tasks = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(tasks)
}

pub fn get(conn: &Connection, id: i64) -> Result<Task> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM task WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Create a task through the sync engine and return it.
fn insert(
    conn: &Connection,
    kind: TaskKind,
    name: &str,
    estimated: Option<i64>,
    parent_uid: Option<String>,
    now: DateTime<Utc>,
) -> Result<Task> {
    let name = validate_name(name)?;
    let estimated = validate_estimate(estimated)?;
    in_tx(conn, |conn| {
        // New list items go to the bottom.
        let ordinal: i64 = if kind.listed() {
            conn.query_row("SELECT COALESCE(MAX(ordinal), -1) + 1 FROM task", [], |r| {
                r.get(0)
            })?
        } else {
            0
        };
        let uid = new_uid();
        record(
            conn,
            Op::CreateTask {
                uid: uid.clone(),
                kind,
                name,
                estimated,
                parent_uid,
                created_at: timestamp(now),
                ordinal,
            },
            now,
        )?;
        get_by_uid(conn, &uid)
    })
}

fn get_by_uid(conn: &Connection, uid: &str) -> Result<Task> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM task WHERE uid = ?1"),
        [uid],
        from_row,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// The global id of a task (what other devices know it by).
fn uid_of(conn: &Connection, id: i64) -> Result<String> {
    conn.query_row("SELECT uid FROM task WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(Error::NotFound)
}

pub fn create(
    conn: &Connection,
    name: &str,
    estimated: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Task> {
    insert(conn, TaskKind::Pomodoro, name, estimated, None, now)
}

/// Record an unplanned item or interruption against `parent_id`; the parent's
/// count follows from it.
pub fn log_disturbance(
    conn: &mut Connection,
    parent_id: i64,
    kind: TaskKind,
    name: &str,
    estimated: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Task> {
    if kind == TaskKind::Pomodoro {
        return Err(Error::Invalid(
            "not an unplanned item or interruption".into(),
        ));
    }
    let estimated = if kind == TaskKind::Interruption {
        None
    } else {
        estimated
    };
    in_tx(conn, |conn| {
        let parent_uid = uid_of(conn, parent_id)?;
        insert(conn, kind, name, estimated, Some(parent_uid), now)
    })
}

pub fn update(
    conn: &Connection,
    id: i64,
    name: &str,
    estimated: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Task> {
    let name = validate_name(name)?;
    let estimated = validate_estimate(estimated)?;
    in_tx(conn, |conn| {
        let current = get(conn, id)?;
        let uid = uid_of(conn, id)?;
        // Only changed fields become events, so an unrelated edit on another
        // device is not overwritten.
        if current.name != name {
            record(
                conn,
                Op::SetName {
                    uid: uid.clone(),
                    name,
                },
                now,
            )?;
        }
        if current.estimated != estimated {
            record(conn, Op::SetEstimate { uid, estimated }, now)?;
        }
        get(conn, id)
    })
}

pub fn set_done(conn: &Connection, id: i64, done: bool, now: DateTime<Utc>) -> Result<Task> {
    in_tx(conn, |conn| {
        let uid = uid_of(conn, id)?;
        let closed_at = done.then(|| timestamp(now));
        record(
            conn,
            Op::SetDone {
                uid,
                done,
                closed_at,
            },
            now,
        )?;
        get(conn, id)
    })
}

pub fn delete(conn: &Connection, id: i64, now: DateTime<Utc>) -> Result<()> {
    in_tx(conn, |conn| {
        let uid = uid_of(conn, id)?;
        record(conn, Op::DeleteTask { uid }, now)?;
        Ok(())
    })
}

/// Set list order to match `ids` (first id = top of the list). Unknown ids are skipped.
pub fn reorder(conn: &mut Connection, ids: &[i64], now: DateTime<Utc>) -> Result<()> {
    in_tx(conn, |conn| {
        for (ordinal, &id) in ids.iter().enumerate() {
            let Ok(task) = get(conn, id) else { continue };
            if task.ordinal != ordinal as i64 {
                let uid = uid_of(conn, id)?;
                record(
                    conn,
                    Op::SetOrdinal {
                        uid,
                        ordinal: ordinal as i64,
                    },
                    now,
                )?;
            }
        }
        Ok(())
    })
}

/// "Clean all done": hide finished tasks from the list (they stay in the statistics).
pub fn hide_done(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    in_tx(conn, |conn| {
        let mut stmt = conn.prepare("SELECT uid FROM task WHERE done = 1 AND visible = 1")?;
        let uids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for uid in &uids {
            record(
                conn,
                Op::SetVisible {
                    uid: uid.clone(),
                    visible: false,
                },
                now,
            )?;
        }
        Ok(uids.len())
    })
}

/// Log a completed pomodoro and credit it to `task_id`, if any.
pub fn record_pomodoro(
    conn: &mut Connection,
    task_id: Option<i64>,
    started: DateTime<Utc>,
    ended: DateTime<Utc>,
) -> Result<()> {
    in_tx(conn, |conn| {
        let task_uid = task_id.map(|id| uid_of(conn, id)).transpose()?;
        record(
            conn,
            Op::LogPomodoro {
                uid: new_uid(),
                task_uid,
                started_at: timestamp(started),
                ended_at: timestamp(ended),
            },
            ended,
        )?;
        Ok(())
    })
}

/// The open (not done) task after `current` in list order, wrapping to the top.
/// Returns `None` when no other open task exists.
pub fn next_open(conn: &Connection, current: Option<i64>) -> Result<Option<i64>> {
    let mut tasks = list(conn)?;
    // Rotate so the search starts just after `current` (which may itself be done).
    if let Some(i) = current.and_then(|id| tasks.iter().position(|t| t.id == id)) {
        tasks.rotate_left(i + 1);
    }
    Ok(tasks
        .iter()
        .find(|t| !t.done && Some(t.id) != current)
        .map(|t| t.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-20T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn names(conn: &Connection) -> Vec<String> {
        list(conn).unwrap().into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn create_appends_to_list() {
        let conn = db::open_in_memory().unwrap();
        create(&conn, "Write release notes", Some(2), now()).unwrap();
        let t = create(&conn, "  Fix #42  ", None, now()).unwrap();
        assert_eq!(t.name, "Fix #42");
        assert_eq!(t.created_at, "2026-10-20T09:00:00Z");
        assert_eq!(names(&conn), ["Write release notes", "Fix #42"]);
    }

    #[test]
    fn rejects_invalid_input() {
        let conn = db::open_in_memory().unwrap();
        assert!(matches!(
            create(&conn, "   ", None, now()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create(&conn, "x", Some(-1), now()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            create(&conn, &"x".repeat(201), None, now()),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            update(&conn, 999, "x", None, now()),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn names_are_stored_verbatim_not_interpolated() {
        let conn = db::open_in_memory().unwrap();
        let t = create(&conn, "Robert'); DROP TABLE task;--", None, now()).unwrap();
        assert_eq!(
            get(&conn, t.id).unwrap().name,
            "Robert'); DROP TABLE task;--"
        );
    }

    #[test]
    fn unplanned_is_listed_and_counted_interruption_only_counted() {
        let mut conn = db::open_in_memory().unwrap();
        let parent = create(&conn, "Main", Some(3), now()).unwrap();
        let u = log_disturbance(
            &mut conn,
            parent.id,
            TaskKind::Unplanned,
            "Hotfix",
            Some(1),
            now(),
        )
        .unwrap();
        let i = log_disturbance(
            &mut conn,
            parent.id,
            TaskKind::Interruption,
            "Phone",
            Some(5),
            now(),
        )
        .unwrap();

        assert_eq!(u.parent_id, Some(parent.id));
        assert_eq!(i.estimated, None, "interruptions carry no estimate");
        let parent = get(&conn, parent.id).unwrap();
        assert_eq!((parent.unplanned, parent.interruptions), (1, 1));
        assert_eq!(names(&conn), ["Main", "Hotfix"]);
    }

    #[test]
    fn disturbance_for_missing_parent_changes_nothing() {
        let mut conn = db::open_in_memory().unwrap();
        let r = log_disturbance(&mut conn, 7, TaskKind::Interruption, "Phone", None, now());
        assert!(matches!(r, Err(Error::NotFound)));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn done_sets_and_clears_closed_at() {
        let conn = db::open_in_memory().unwrap();
        let t = create(&conn, "A", None, now()).unwrap();
        let t = set_done(&conn, t.id, true, now()).unwrap();
        assert!(t.done);
        assert_eq!(t.closed_at.as_deref(), Some("2026-10-20T09:00:00Z"));
        let t = set_done(&conn, t.id, false, now()).unwrap();
        assert!(!t.done);
        assert_eq!(t.closed_at, None);
    }

    #[test]
    fn hide_done_removes_only_done_from_list() {
        let conn = db::open_in_memory().unwrap();
        let a = create(&conn, "A", None, now()).unwrap();
        create(&conn, "B", None, now()).unwrap();
        set_done(&conn, a.id, true, now()).unwrap();
        assert_eq!(hide_done(&conn, now()).unwrap(), 1);
        assert_eq!(names(&conn), ["B"]);
        assert!(
            get(&conn, a.id).is_ok(),
            "hidden tasks are kept for statistics"
        );
    }

    #[test]
    fn reorder_follows_given_ids() {
        let mut conn = db::open_in_memory().unwrap();
        let a = create(&conn, "A", None, now()).unwrap();
        let b = create(&conn, "B", None, now()).unwrap();
        let c = create(&conn, "C", None, now()).unwrap();
        reorder(&mut conn, &[c.id, a.id, b.id], now()).unwrap();
        assert_eq!(names(&conn), ["C", "A", "B"]);
        create(&conn, "D", None, now()).unwrap();
        assert_eq!(names(&conn), ["C", "A", "B", "D"]);
    }

    #[test]
    fn record_pomodoro_credits_task_and_logs() {
        let mut conn = db::open_in_memory().unwrap();
        let t = create(&conn, "A", Some(2), now()).unwrap();
        record_pomodoro(&mut conn, Some(t.id), now(), now()).unwrap();
        record_pomodoro(&mut conn, None, now(), now()).unwrap();
        assert_eq!(get(&conn, t.id).unwrap().pomodoros, 1);
        let logged: i64 = conn
            .query_row("SELECT COUNT(*) FROM pomodoro_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 2);
    }

    #[test]
    fn delete_keeps_log_and_children() {
        let mut conn = db::open_in_memory().unwrap();
        let t = create(&conn, "A", None, now()).unwrap();
        let u = log_disturbance(&mut conn, t.id, TaskKind::Unplanned, "U", None, now()).unwrap();
        record_pomodoro(&mut conn, Some(t.id), now(), now()).unwrap();
        delete(&conn, t.id, now()).unwrap();
        assert_eq!(get(&conn, u.id).unwrap().parent_id, None);
        let orphan: Option<i64> = conn
            .query_row("SELECT task_id FROM pomodoro_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphan, None);
        assert!(matches!(delete(&conn, t.id, now()), Err(Error::NotFound)));
    }

    #[test]
    fn next_open_skips_done_and_wraps() {
        let conn = db::open_in_memory().unwrap();
        let a = create(&conn, "A", None, now()).unwrap();
        let b = create(&conn, "B", None, now()).unwrap();
        let c = create(&conn, "C", None, now()).unwrap();
        set_done(&conn, b.id, true, now()).unwrap();
        assert_eq!(next_open(&conn, Some(a.id)).unwrap(), Some(c.id));
        assert_eq!(next_open(&conn, Some(c.id)).unwrap(), Some(a.id));
        // A `current` that was just marked done still moves forward from its position.
        assert_eq!(next_open(&conn, Some(b.id)).unwrap(), Some(c.id));
        assert_eq!(next_open(&conn, None).unwrap(), Some(a.id));
        set_done(&conn, c.id, true, now()).unwrap();
        assert_eq!(next_open(&conn, Some(a.id)).unwrap(), None);
    }
}
