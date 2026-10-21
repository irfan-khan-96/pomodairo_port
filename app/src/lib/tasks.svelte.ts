import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Mirrors `Task` and `TaskState` in src-tauri/src/tasks.rs and task_service.rs.
export type TaskKind = "pomodoro" | "unplanned" | "interruption";

export interface Task {
  id: number;
  name: string;
  kind: TaskKind;
  parent_id: number | null;
  estimated: number | null;
  pomodoros: number;
  unplanned: number;
  interruptions: number;
  done: boolean;
  ordinal: number;
  created_at: string;
  closed_at: string | null;
}

interface TaskState {
  tasks: Task[];
  active_id: number | null;
}

/** Reactive view of the task list. Rust owns the data; every change comes back as a new state. */
class TaskStore {
  tasks = $state<Task[]>([]);
  activeId = $state<number | null>(null);
  error = $state<string | null>(null);

  active = $derived(this.tasks.find((t) => t.id === this.activeId) ?? null);

  async connect(): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    const unlisten = await listen<TaskState>("tasks://changed", (e) => this.apply(e.payload));
    this.apply(await invoke<TaskState>("tasks_state"));
    return unlisten;
  }

  create = (name: string, estimated: number | null) => this.call("task_create", { name, estimated });
  update = (id: number, name: string, estimated: number | null) =>
    this.call("task_update", { id, name, estimated });
  select = (id: number) => this.call("task_select", { id });
  setDone = (id: number, done: boolean) => this.call("task_set_done", { id, done });
  remove = (id: number) => this.call("task_delete", { id });
  reorder = (ids: number[]) => this.call("task_reorder", { ids });
  hideDone = () => this.call("tasks_hide_done");
  logDisturbance = (kind: "unplanned" | "interruption", name: string, estimated: number | null) =>
    this.call("task_log_disturbance", { kind, name, estimated });

  private apply(state: TaskState) {
    this.tasks = state.tasks;
    this.activeId = state.active_id;
  }

  private async call(cmd: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      this.apply(await invoke<TaskState>(cmd, args));
      this.error = null;
      return true;
    } catch (e) {
      this.error = String(e);
      return false;
    }
  }
}

export const tasks = new TaskStore();

/** `Write docs (1/2)`, the label under the clock. */
export function shortDescription(t: Task): string {
  return `${t.name} (${t.pomodoros}/${t.estimated ?? 0})`;
}

export function isToday(iso: string): boolean {
  return new Date(iso).toDateString() === new Date().toDateString();
}

/** Tab-separated table of the list, for "Copy to clipboard". */
export function tasksAsTsv(list: Task[]): string {
  const rows = list.map((t) =>
    [t.name, t.estimated ?? "", t.pomodoros, t.unplanned, t.interruptions, t.done ? "done" : ""].join("\t"),
  );
  return ["Name\tEstimated\tPomodoros\tUnplanned\tInterruptions\tStatus", ...rows].join("\n");
}
