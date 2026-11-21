import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Mirrors `Settings` in src-tauri/src/settings.rs.
export interface Settings {
  timing: {
    pomodoro_minutes: number;
    short_break_minutes: number;
    long_break_minutes: number;
    long_break_interval: number;
  };
  sound: {
    enabled: boolean;
    volume: number;
    fade_ticking: boolean;
    alarm_file: string | null;
    ticking_file: string | null;
  };
  general: {
    update_taskbar: boolean;
    url_to_open: string;
    notifications: boolean;
    keep_in_tray: boolean;
    launch_at_login: boolean;
    check_updates: boolean;
  };
}

export interface DatabaseLocation {
  path: string;
  default_dir: string;
  overridden: boolean;
}

export type SettingsTab = "general" | "timing" | "sounds" | "devices" | "advanced";
export const TABS: SettingsTab[] = ["general", "timing", "sounds", "devices", "advanced"];

export const DEFAULT_SETTINGS: Settings = {
  timing: { pomodoro_minutes: 25, short_break_minutes: 5, long_break_minutes: 15, long_break_interval: 4 },
  sound: { enabled: true, volume: 100, fade_ticking: true, alarm_file: null, ticking_file: null },
  general: {
    update_taskbar: true,
    url_to_open: "",
    notifications: true,
    keep_in_tray: false,
    launch_at_login: false,
    check_updates: true,
  },
};

/** Saved settings, kept current across windows. */
class SettingsStore {
  current = $state<Settings>(structuredClone(DEFAULT_SETTINGS));

  async connect(): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    const unlisten = await listen<Settings>("settings://changed", (e) => (this.current = e.payload));
    this.current = await invoke<Settings>("settings_get");
    return unlisten;
  }

  /** Validate and save in Rust; rejects with a readable message. */
  async save(settings: Settings): Promise<void> {
    if (!isTauri()) {
      this.current = settings;
      return;
    }
    this.current = await invoke<Settings>("settings_save", { settings });
  }
}

export const settings = new SettingsStore();

export function openSettings(tab: SettingsTab = "general") {
  if (isTauri()) invoke("open_settings_window", { tab }).catch((e) => console.error(e));
}

/** Ticket number in a task name like "Fix login #123", for "URL to open". */
export function ticketNumber(name: string): string | null {
  return /#(\d+)/.exec(name)?.[1] ?? null;
}
