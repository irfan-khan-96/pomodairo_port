-- LAN sync: this device's certificate (public) and the devices it is paired with.
-- The matching private key is kept in the OS keychain (or a private file), not here.
ALTER TABLE sync_state ADD COLUMN cert TEXT;  -- hex DER

CREATE TABLE sync_peer (
  device      TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  fingerprint TEXT NOT NULL UNIQUE,  -- SHA-256 of the peer's certificate (pinned)
  paired_at   TEXT NOT NULL,
  last_sync   TEXT,
  last_addr   TEXT
);
