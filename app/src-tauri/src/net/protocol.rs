//! Messages between devices: length-prefixed JSON frames over TLS, with size and
//! time limits so a misbehaving peer can't exhaust memory or hang a connection.
//!
//! Pairing shows the same 6-digit code on both screens. To stop an attacker in
//! the middle from trying many keys until the codes match, the initiator commits
//! to its random nonce (sends its hash) before seeing the responder's nonce, as
//! in Bluetooth numeric comparison. The code then depends on both certificates
//! and both nonces, and can't be steered: a forged match is a 1-in-a-million guess.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::identity::hex;
use crate::error::{Error, Result};
use crate::sync::engine::Event;

pub const PROTOCOL: u32 = 1;
/// Largest accepted frame (a first sync of years of history fits comfortably).
pub const MAX_FRAME: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Pair,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Msg {
    Hello {
        protocol: u32,
        device: String,
        name: String,
        mode: Mode,
    },
    Reject {
        reason: String,
    },
    Commit {
        hash: String,
    },
    Nonce {
        nonce: String,
    },
    Reveal {
        nonce: String,
    },
    Confirm {
        accept: bool,
    },
    Vector {
        vv: BTreeMap<String, i64>,
    },
    Events {
        events: Vec<Event>,
    },
    /// Sent after applying the received events: the sync is only counted as
    /// delivered once both sides have said so.
    Applied,
}

pub fn net_err(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{what}: {e}"))
}

pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &Msg) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME {
        return Err(Error::Invalid("message too large to send".into()));
    }
    let len = u32::try_from(body.len()).expect("checked above");
    let io = async {
        w.write_all(&len.to_be_bytes()).await?;
        w.write_all(&body).await?;
        w.flush().await
    };
    tokio::time::timeout(Duration::from_secs(60), io)
        .await
        .map_err(|_| Error::Invalid("timed out sending".into()))?
        .map_err(|e| net_err("connection lost", e))
}

pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R, wait: Duration) -> Result<Msg> {
    let io = async {
        let mut len = [0u8; 4];
        r.read_exact(&mut len)
            .await
            .map_err(|e| net_err("connection lost", e))?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME {
            return Err(Error::Invalid("peer sent an oversized message".into()));
        }
        let mut body = vec![0u8; len];
        r.read_exact(&mut body)
            .await
            .map_err(|e| net_err("connection lost", e))?;
        Ok(serde_json::from_slice(&body)?)
    };
    tokio::time::timeout(wait, io)
        .await
        .map_err(|_| Error::Invalid("peer did not answer in time".into()))?
}

/// Hex SHA-256 commitment to a nonce.
pub fn commitment(nonce: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"pomodairo-commit-v1");
    h.update(nonce);
    hex(&h.finalize())
}

/// The 6-digit pairing code both screens show. `client_fp` is the initiator's
/// certificate fingerprint, `server_fp` the responder's.
pub fn pairing_code(
    client_fp: &str,
    server_fp: &str,
    client_nonce: &[u8],
    server_nonce: &[u8],
) -> String {
    let mut h = Sha256::new();
    h.update(b"pomodairo-sas-v1");
    h.update(client_fp.as_bytes());
    h.update(server_fp.as_bytes());
    h.update(client_nonce);
    h.update(server_nonce);
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{n:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msg = Msg::Confirm { accept: true };
        write_msg(&mut a, &msg).await.unwrap();
        assert_eq!(read_msg(&mut b, Duration::from_secs(1)).await.unwrap(), msg);
    }

    #[tokio::test]
    async fn oversized_frames_are_refused_before_reading_them() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        let err = read_msg(&mut b, Duration::from_secs(1)).await.unwrap_err();
        assert!(err.to_string().contains("oversized"));
    }

    #[tokio::test]
    async fn a_silent_peer_times_out() {
        let (_a, mut b) = tokio::io::duplex(64);
        let err = read_msg(&mut b, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("in time"));
    }

    #[test]
    fn pairing_code_depends_on_everything() {
        let base = pairing_code("aa", "bb", b"n1", b"n2");
        assert_eq!(base.len(), 6);
        assert_eq!(
            base,
            pairing_code("aa", "bb", b"n1", b"n2"),
            "deterministic"
        );
        for other in [
            pairing_code("ab", "bb", b"n1", b"n2"),
            pairing_code("aa", "bc", b"n1", b"n2"),
            pairing_code("aa", "bb", b"n3", b"n2"),
            pairing_code("aa", "bb", b"n1", b"n4"),
            pairing_code("bb", "aa", b"n1", b"n2"),
        ] {
            assert_ne!(other, base);
        }
        assert_ne!(commitment(b"x"), commitment(b"y"));
    }
}
