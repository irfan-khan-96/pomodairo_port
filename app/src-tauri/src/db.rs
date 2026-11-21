//! SQLite connection setup and schema migrations.

use std::path::Path;

use rusqlite::Connection;

pub const DATABASE_FILE: &str = "pomodairo.db";

/// Migrations in order; `PRAGMA user_version` records how many have been applied.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_init.sql"),
    include_str!("../migrations/002_sync.sql"),
    include_str!("../migrations/003_peers.sql"),
    include_str!("../migrations/004_handoff.sql"),
];

pub fn open(path: &Path) -> crate::error::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    backup_before_upgrade(&conn, path)?;
    init(conn)
}

/// Before migrating an existing database, keep a copy next to it
/// (`pomodairo.db.v1.bak` for a database at schema version 1).
fn backup_before_upgrade(conn: &Connection, path: &Path) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version == 0 || version as usize >= MIGRATIONS.len() {
        return Ok(());
    }
    let backup = path.with_extension(format!("db.v{version}.bak"));
    if !backup.exists() {
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()])?;
    }
    Ok(())
}

#[cfg(test)]
pub fn open_in_memory() -> crate::error::Result<Connection> {
    init(Connection::open_in_memory()?)
}

pub(crate) fn init(mut conn: Connection) -> crate::error::Result<Connection> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&mut conn)?;
    // Turn data from before sync into events (no-op once done).
    crate::sync::engine::bootstrap(&conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(applied as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once() {
        let mut conn = open_in_memory().unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        // Re-running is a no-op rather than a "table already exists" error.
        migrate(&mut conn).unwrap();
    }

    /// Upgrading a schema-1 database file, opened twice as the app does (task
    /// service and settings): a backup is kept, data is unchanged, and the
    /// pre-sync data is turned into events exactly once.
    #[test]
    fn upgrading_a_database_file_keeps_a_backup_and_the_data() {
        let dir = std::env::temp_dir().join(format!("pomodairo-upgrade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(DATABASE_FILE);
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(MIGRATIONS[0]).unwrap();
            old.pragma_update(None, "user_version", 1).unwrap();
            old.execute_batch(
                "INSERT INTO task (name, kind, estimated, pomodoros, done, closed_at, created_at)
                   VALUES ('Kept', 'pomodoro', 2, 1, 1, '2026-10-02T10:00:00Z', '2026-10-01T09:00:00Z');
                 INSERT INTO pomodoro_log (task_id, started_at, ended_at)
                   VALUES (1, '2026-10-01T09:00:00Z', '2026-10-01T09:25:00Z');",
            )
            .unwrap();
        }
        let tasks_conn = open(&path).unwrap();
        let settings_conn = open(&path).unwrap();

        assert!(dir.join("pomodairo.db.v1.bak").exists(), "backup kept");
        let backup = Connection::open(dir.join("pomodairo.db.v1.bak")).unwrap();
        let backup_version: i64 = backup
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(backup_version, 1, "backup is the untouched old database");

        let row: (String, i64, bool, Option<String>) = tasks_conn
            .query_row(
                "SELECT name, pomodoros, done, closed_at FROM task WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            ("Kept".into(), 1, true, Some("2026-10-02T10:00:00Z".into()))
        );
        let events: i64 = settings_conn
            .query_row("SELECT COUNT(*) FROM sync_event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            events, 3,
            "create + done + pomodoro, not duplicated by the second open"
        );

        drop((tasks_conn, settings_conn, backup));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
