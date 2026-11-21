-- Hand-off indicator: how many of this device's own changes each paired device
-- has confirmed applying (this device's event sequence number at that sync).
ALTER TABLE sync_peer ADD COLUMN acked_seq INTEGER NOT NULL DEFAULT 0;
