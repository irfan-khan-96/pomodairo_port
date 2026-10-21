//! Key/value settings stored in the `config` table.

use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::db;
use crate::error::Result;

pub struct ConfigStore {
    conn: Mutex<Connection>,
}

impl ConfigStore {
    /// Opens its own connection to the shared database file (safe with WAL).
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self::from_conn(db::open(path)?))
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::from_conn(db::open_in_memory().unwrap())
    }

    fn from_conn(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .unwrap_or_else(|e| {
            eprintln!("config: failed to read {key}: {e}");
            None
        })
    }

    /// The value parsed as `T`, or `None` if missing or unparsable.
    pub fn get_parsed<T: FromStr>(&self, key: &str) -> Option<T> {
        self.get(key)?.parse().ok()
    }

    /// Write several values in one transaction.
    pub fn set_many(&self, values: &[(&str, String)]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt =
                tx.prepare("INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)")?;
            for (key, value) in values {
                stmt.execute(params![key, value])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn set(&self, key: &str, value: impl ToString) -> Result<()> {
        self.set_many(&[(key, value.to_string())])
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM config WHERE key = ?1", [key])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_overwrite_remove() {
        let cfg = ConfigStore::in_memory();
        assert_eq!(cfg.get("a"), None);
        cfg.set("a", 1).unwrap();
        cfg.set("a", "two").unwrap();
        assert_eq!(cfg.get("a").as_deref(), Some("two"));
        cfg.remove("a").unwrap();
        assert_eq!(cfg.get("a"), None);
    }

    #[test]
    fn parsed_values() {
        let cfg = ConfigStore::in_memory();
        cfg.set_many(&[
            ("b", "true".into()),
            ("n", "12.5".into()),
            ("bad", "x".into()),
        ])
        .unwrap();
        assert_eq!(cfg.get_parsed::<bool>("b"), Some(true));
        assert_eq!(cfg.get_parsed::<f64>("n"), Some(12.5));
        assert_eq!(cfg.get_parsed::<f64>("bad"), None);
        assert_eq!(cfg.get_parsed::<i32>("missing"), None);
    }

    #[test]
    fn values_are_not_interpolated() {
        let cfg = ConfigStore::in_memory();
        cfg.set("k'); DROP TABLE config;--", "v'").unwrap();
        assert_eq!(cfg.get("k'); DROP TABLE config;--").as_deref(), Some("v'"));
    }
}
