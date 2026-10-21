//! SQLite connection setup and schema migrations.

use std::path::Path;

use rusqlite::Connection;

pub const DATABASE_FILE: &str = "pomodairo.db";

/// Migrations in order; `PRAGMA user_version` records how many have been applied.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/001_init.sql")];

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    init(Connection::open(path)?)
}

#[cfg(test)]
pub fn open_in_memory() -> rusqlite::Result<Connection> {
    init(Connection::open_in_memory()?)
}

fn init(mut conn: Connection) -> rusqlite::Result<Connection> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&mut conn)?;
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
}
