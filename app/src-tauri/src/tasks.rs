//! Task storage: the task list, unplanned items, interruptions and completed pomodoros.
//!
//! Functions take the connection and the current time explicitly so they can be
//! tested against an in-memory database.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

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
    fn as_str(self) -> &'static str {
        match self {
            TaskKind::Pomodoro => "pomodoro",
            TaskKind::Unplanned => "unplanned",
            TaskKind::Interruption => "interruption",
        }
    }

    fn parse(s: &str) -> rusqlite::Result<Self> {
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

fn validate_name(name: &str) -> Result<String> {
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

fn validate_estimate(estimated: Option<i64>) -> Result<Option<i64>> {
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
         ORDER BY ordinal, id"
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

fn insert(
    conn: &Connection,
    kind: TaskKind,
    name: &str,
    estimated: Option<i64>,
    parent_id: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Task> {
    let name = validate_name(name)?;
    let estimated = validate_estimate(estimated)?;
    // New list items go to the bottom.
    let ordinal: i64 = if kind.listed() {
        conn.query_row("SELECT COALESCE(MAX(ordinal), -1) + 1 FROM task", [], |r| {
            r.get(0)
        })?
    } else {
        0
    };
    conn.execute(
        "INSERT INTO task (name, kind, parent_id, estimated, ordinal, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            name,
            kind.as_str(),
            parent_id,
            estimated,
            ordinal,
            timestamp(now)
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn create(
    conn: &Connection,
    name: &str,
    estimated: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Task> {
    insert(conn, TaskKind::Pomodoro, name, estimated, None, now)
}

/// Record an unplanned item or interruption against `parent_id` and bump the parent's counter.
pub fn log_disturbance(
    conn: &mut Connection,
    parent_id: i64,
    kind: TaskKind,
    name: &str,
    estimated: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Task> {
    let counter = match kind {
        TaskKind::Unplanned => "unplanned",
        TaskKind::Interruption => "interruptions",
        TaskKind::Pomodoro => {
            return Err(Error::Invalid(
                "not an unplanned item or interruption".into(),
            ));
        }
    };
    let estimated = if kind == TaskKind::Interruption {
        None
    } else {
        estimated
    };
    let tx = conn.transaction()?;
    get(&tx, parent_id)?;
    let item = insert(&tx, kind, name, estimated, Some(parent_id), now)?;
    tx.execute(
        &format!("UPDATE task SET {counter} = {counter} + 1 WHERE id = ?1"),
        [parent_id],
    )?;
    tx.commit()?;
    Ok(item)
}

pub fn update(conn: &Connection, id: i64, name: &str, estimated: Option<i64>) -> Result<Task> {
    let name = validate_name(name)?;
    let estimated = validate_estimate(estimated)?;
    let changed = conn.execute(
        "UPDATE task SET name = ?1, estimated = ?2 WHERE id = ?3",
        params![name, estimated, id],
    )?;
    if changed == 0 {
        return Err(Error::NotFound);
    }
    get(conn, id)
}

pub fn set_done(conn: &Connection, id: i64, done: bool, now: DateTime<Utc>) -> Result<Task> {
    let closed_at = done.then(|| timestamp(now));
    let changed = conn.execute(
        "UPDATE task SET done = ?1, closed_at = ?2 WHERE id = ?3",
        params![done, closed_at, id],
    )?;
    if changed == 0 {
        return Err(Error::NotFound);
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    if conn.execute("DELETE FROM task WHERE id = ?1", [id])? == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Set list order to match `ids` (first id = top of the list).
pub fn reorder(conn: &mut Connection, ids: &[i64]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE task SET ordinal = ?1 WHERE id = ?2")?;
        for (ordinal, id) in ids.iter().enumerate() {
            stmt.execute(params![ordinal as i64, id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// "Clean all done": hide finished tasks from the list (they stay in the statistics).
pub fn hide_done(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE task SET visible = 0 WHERE done = 1 AND visible = 1",
        [],
    )?)
}

/// Log a completed pomodoro and credit it to `task_id`, if any.
pub fn record_pomodoro(
    conn: &mut Connection,
    task_id: Option<i64>,
    started: DateTime<Utc>,
    ended: DateTime<Utc>,
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO pomodoro_log (task_id, started_at, ended_at) VALUES (?1, ?2, ?3)",
        params![task_id, timestamp(started), timestamp(ended)],
    )?;
    if let Some(id) = task_id {
        tx.execute(
            "UPDATE task SET pomodoros = pomodoros + 1 WHERE id = ?1",
            [id],
        )?;
    }
    tx.commit()?;
    Ok(())
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
            update(&conn, 999, "x", None),
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
        assert_eq!(hide_done(&conn).unwrap(), 1);
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
        reorder(&mut conn, &[c.id, a.id, b.id]).unwrap();
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
        delete(&conn, t.id).unwrap();
        assert_eq!(get(&conn, u.id).unwrap().parent_id, None);
        let orphan: Option<i64> = conn
            .query_row("SELECT task_id FROM pomodoro_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphan, None);
        assert!(matches!(delete(&conn, t.id), Err(Error::NotFound)));
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
