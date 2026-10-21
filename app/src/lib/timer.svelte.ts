import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Mirrors `TimerSnapshot` in src-tauri/src/timer.rs.
export type Phase = "idle" | "pomodoro" | "time_out" | "break" | "break_over";
export type BreakKind = "short" | "long";

export interface TimerSnapshot {
  phase: Phase;
  break_kind: BreakKind | null;
  remaining_secs: number;
  total_secs: number;
  completed: number;
}

const IDLE: TimerSnapshot = {
  phase: "idle",
  break_kind: null,
  remaining_secs: 25 * 60,
  total_secs: 25 * 60,
  completed: 0,
};

/** Reactive view of the Rust timer. Rust owns the state; this only mirrors it. */
class TimerStore {
  snapshot = $state<TimerSnapshot>(IDLE);

  async connect(): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    const unlisten = await listen<TimerSnapshot>("timer://snapshot", (e) => {
      this.snapshot = e.payload;
    });
    this.snapshot = await invoke<TimerSnapshot>("timer_snapshot");
    return unlisten;
  }

  async start(): Promise<void> {
    if (isTauri()) this.snapshot = await invoke<TimerSnapshot>("timer_start");
  }

  async stop(): Promise<void> {
    if (isTauri()) this.snapshot = await invoke<TimerSnapshot>("timer_stop");
  }
}

export const timer = new TimerStore();

export function formatClock(totalSecs: number): string {
  const m = Math.floor(totalSecs / 60);
  const s = totalSecs % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}
