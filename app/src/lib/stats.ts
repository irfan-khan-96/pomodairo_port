import { invoke, isTauri } from "@tauri-apps/api/core";

// Mirrors src-tauri/src/stats.rs.
export interface DateRange {
  from: string; // YYYY-MM-DD, local
  to: string;
}

export interface CompletedTask {
  name: string;
  kind: "pomodoro" | "unplanned";
  closed_at: string;
  estimated: number | null;
  pomodoros: number;
  delta: number | null;
  interruptions: number;
}

export interface Disturbance {
  name: string;
  kind: "interruption" | "unplanned";
  created_at: string;
  task: string | null;
}

export interface DayCount {
  day: string;
  pomodoros: number;
  interruptions: number;
  unplanned: number;
  tasks_done: number;
}

export interface WeekFactor {
  week: string;
  week_start: string;
  estimated: number;
  pomodoros: number;
  factor: number | null;
  delta: number;
  interruptions: number;
}

export interface Hashtags {
  tasks: string[];
  disturbances: string[];
}

async function call<T>(cmd: string, args: Record<string, unknown>, fallback: T): Promise<T> {
  return isTauri() ? invoke<T>(cmd, args) : fallback;
}

export const fetchCompleted = (range: DateRange, filter: string) =>
  call<CompletedTask[]>("stats_completed", { range, filter }, []);
export const fetchDisturbances = (range: DateRange, filter: string) =>
  call<Disturbance[]>("stats_disturbances", { range, filter }, []);
export const fetchPerDay = (range: DateRange) => call<DayCount[]>("stats_per_day", { range }, []);
export const fetchReality = () => call<WeekFactor[]>("stats_reality", {}, []);
export const fetchHashtags = () => call<Hashtags>("stats_hashtags", {}, { tasks: [], disturbances: [] });

export function openStats() {
  if (isTauri()) invoke("open_stats_window").catch((e) => console.error(e));
}

/** Local calendar date as YYYY-MM-DD. */
export function isoDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The last `days` days, ending today. */
export function lastDays(days: number): DateRange {
  const to = new Date();
  const from = new Date(to);
  from.setDate(to.getDate() - (days - 1));
  return { from: isoDay(from), to: isoDay(to) };
}

/** Stored UTC timestamp shown as local "YYYY-MM-DD HH:mm". */
export function localDateTime(iso: string): string {
  const d = new Date(iso);
  return `${isoDay(d)} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** "Mon 20 Oct" for a YYYY-MM-DD day. */
export function shortDay(day: string): string {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
}

/** Tab-separated table for the clipboard (pastes cleanly into spreadsheets). */
export function toTsv(headers: string[], rows: (string | number | null)[][]): string {
  const clean = (v: string | number | null) => String(v ?? "").replace(/[\t\n\r]+/g, " ");
  return [headers, ...rows].map((r) => r.map(clean).join("\t")).join("\n");
}
