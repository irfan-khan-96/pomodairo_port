//! Paired devices, as stored in `sync_peer` (device-local, never synced).

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::error::Result;
use crate::tasks::timestamp;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Peer {
    pub device: String,
    pub name: String,
    /// SHA-256 of the peer's certificate; connections presenting anything else are refused.
    pub fingerprint: String,
    pub paired_at: String,
    pub last_sync: Option<String>,
    pub last_addr: Option<String>,
    /// This device's own changes up to this sequence number are on the peer.
    pub acked_seq: i64,
}

const COLUMNS: &str = "device, name, fingerprint, paired_at, last_sync, last_addr, acked_seq";

fn from_row(r: &Row) -> rusqlite::Result<Peer> {
    Ok(Peer {
        device: r.get(0)?,
        name: r.get(1)?,
        fingerprint: r.get(2)?,
        paired_at: r.get(3)?,
        last_sync: r.get(4)?,
        last_addr: r.get(5)?,
        acked_seq: r.get(6)?,
    })
}

pub fn list(conn: &Connection) -> Result<Vec<Peer>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM sync_peer ORDER BY name, device"
    ))?;
    let peers = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(peers)
}

pub fn by_fingerprint(conn: &Connection, fingerprint: &str) -> Result<Option<Peer>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM sync_peer WHERE fingerprint = ?1"),
            [fingerprint],
            from_row,
        )
        .optional()?)
}

pub fn by_device(conn: &Connection, device: &str) -> Result<Option<Peer>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM sync_peer WHERE device = ?1"),
            [device],
            from_row,
        )
        .optional()?)
}

/// Pair with a device (or re-pair, replacing its old certificate).
pub fn save(
    conn: &Connection,
    device: &str,
    name: &str,
    fingerprint: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "DELETE FROM sync_peer WHERE fingerprint = ?1 AND device != ?2",
        params![fingerprint, device],
    )?;
    conn.execute(
        "INSERT INTO sync_peer (device, name, fingerprint, paired_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (device) DO UPDATE SET name = excluded.name, fingerprint = excluded.fingerprint,
           paired_at = excluded.paired_at, last_sync = NULL, acked_seq = 0",
        params![device, name, fingerprint, timestamp(now)],
    )?;
    Ok(())
}

/// Record a completed sync; `acked_seq` is this device's own sequence number
/// that the peer has now confirmed applying.
pub fn mark_synced(
    conn: &Connection,
    device: &str,
    addr: Option<&str>,
    acked_seq: i64,
    now: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "UPDATE sync_peer SET last_sync = ?2, last_addr = COALESCE(?3, last_addr),
           acked_seq = MAX(acked_seq, ?4) WHERE device = ?1",
        params![device, timestamp(now), addr, acked_seq],
    )?;
    Ok(())
}

/// How many of this device's own changes `peer` hasn't confirmed yet.
pub fn pending(conn: &Connection, me: &str, peer: &Peer) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM sync_event WHERE device = ?1 AND seq > ?2",
        params![me, peer.acked_seq],
        |r| r.get(0),
    )?)
}

pub fn remove(conn: &Connection, device: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM sync_peer WHERE device = ?1", [device])? > 0)
}
