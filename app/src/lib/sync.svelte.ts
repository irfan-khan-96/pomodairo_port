import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Mirrors `SyncStatus` in src-tauri/src/sync_service.rs.
export interface PeerStatus {
  device: string;
  name: string;
  last_sync: string | null;
  online: boolean;
  /// Changes made on this device that the peer doesn't have yet.
  pending: number;
}

export interface SyncStatus {
  device_id: string;
  name: string;
  pairing_open: boolean;
  pending_total: number;
  peers: PeerStatus[];
  /// Unpaired devices nearby that are waiting to pair.
  nearby: { device: string; name: string }[];
  last_error: string | null;
}

export interface PairingPrompt {
  code: string;
  peer_name: string;
  timeout_secs: number;
}

/** Device sync status, kept current by events from Rust. */
class SyncStore {
  status = $state<SyncStatus | null>(null);
  /** The pairing code awaiting confirmation, if any. */
  prompt = $state<PairingPrompt | null>(null);
  /** Shown while waiting for the other device to connect. */
  waitingUntil = $state<number | null>(null);
  busy = $state(false);
  error = $state<string | null>(null);

  async connect(): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    const offs = await Promise.all([
      listen<SyncStatus>("sync://status", (e) => (this.status = e.payload)),
      listen<PairingPrompt>("sync://pairing", (e) => {
        this.prompt = e.payload;
        this.waitingUntil = null;
      }),
    ]);
    this.status = await invoke<SyncStatus>("sync_status");
    return () => offs.forEach((off) => off());
  }

  private async run(cmd: string, args?: Record<string, unknown>) {
    this.error = null;
    try {
      const result = await invoke<SyncStatus | null>(cmd, args);
      if (result) this.status = result;
    } catch (e) {
      this.error = String(e);
    }
  }

  /** Make this device pairable for two minutes. */
  async openPairing() {
    await this.run("sync_open_pairing");
    if (!this.error) this.waitingUntil = Date.now() + 120_000;
  }

  async cancelPairing() {
    this.waitingUntil = null;
    this.prompt = null;
    await this.run("sync_cancel_pairing");
  }

  discover = () => this.run("sync_discover");

  async pairWith(device: string) {
    this.busy = true;
    await this.run("sync_pair_with", { device });
    this.busy = false;
    this.prompt = null;
  }

  async confirm(accept: boolean) {
    this.prompt = null;
    this.waitingUntil = null;
    await invoke("sync_confirm_pairing", { accept }).catch(() => {});
  }

  unpair = (device: string) => this.run("sync_unpair", { device });
  syncNow = () => invoke("sync_now").catch(() => {});
}

export const sync = new SyncStore();

/** "just now", "5 min ago", "yesterday 14:05"… */
export function ago(iso: string | null): string {
  if (!iso) return "never";
  const then = new Date(iso);
  const secs = Math.round((Date.now() - then.getTime()) / 1000);
  if (secs < 45) return "just now";
  if (secs < 3600) return `${Math.round(secs / 60)} min ago`;
  if (secs < 86400 && then.getDate() === new Date().getDate()) return `today ${then.toTimeString().slice(0, 5)}`;
  return then.toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function changes(n: number): string {
  return `${n} change${n === 1 ? "" : "s"}`;
}
