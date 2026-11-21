import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message } from "@tauri-apps/plugin-dialog";

// Mirrors `UpdateStatus` in src-tauri/src/updater_service.rs.
export interface AvailableUpdate {
  version: string;
  current: string;
  notes: string | null;
}

export interface UpdateStatus {
  current: string;
  available: AvailableUpdate | null;
  installing: boolean;
}

/** App updates from GitHub Releases, checked by Rust. */
class UpdateStore {
  status = $state<UpdateStatus | null>(null);

  async connect(): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    const off = await listen<UpdateStatus>("update://status", (e) => (this.status = e.payload));
    this.status = await invoke<UpdateStatus>("update_status");
    return off;
  }

  /** Ask, then download, install and restart. */
  install = async () => {
    const u = this.status?.available;
    if (!u || this.status?.installing) return;
    const notes = u.notes ? `\n\n${u.notes.slice(0, 600)}` : "";
    const ok = await ask(
      `Pomodairo ${u.version} is available (you have ${u.current}).${notes}\n\nInstall it now? Pomodairo will restart.`,
      { title: "Update available", okLabel: "Install and restart", cancelLabel: "Later" },
    );
    if (!ok) return;
    try {
      await invoke("update_install");
    } catch (e) {
      await message(String(e), { title: "Update", kind: "error" });
    }
  };

  /** "Check for updates..." in the menu. */
  checkNow = async () => {
    try {
      const found = await invoke<AvailableUpdate | null>("update_check");
      if (found) await this.install();
      else
        await message(`You're up to date (Pomodairo ${this.status?.current ?? ""}).`, {
          title: "Check for updates",
        });
    } catch (e) {
      await message(String(e), { title: "Check for updates", kind: "warning" });
    }
  };
}

export const updates = new UpdateStore();
