<script lang="ts">
  import { Menu } from "@tauri-apps/api/menu";
  import { isTauri } from "@tauri-apps/api/core";
  import { isToday, tasks, tasksAsTsv, type Task } from "$lib/tasks.svelte";

  let {
    onadd,
    onedit,
    onshow,
  }: {
    onadd: () => void;
    onedit: (task: Task) => void;
    onshow: (task: Task) => void;
  } = $props();

  // ---- Drag to reorder (pointer events, so it behaves the same on every webview) ----
  const DRAG_THRESHOLD = 4;
  let drag = $state<{ id: number; startY: number; dy: number; moved: boolean; rowHeight: number } | null>(null);

  const order = $derived.by(() => {
    if (!drag?.moved) return tasks.tasks;
    const from = tasks.tasks.findIndex((t) => t.id === drag!.id);
    const to = Math.max(0, Math.min(tasks.tasks.length - 1, from + Math.round(drag.dy / drag.rowHeight)));
    const list = [...tasks.tasks];
    list.splice(to, 0, ...list.splice(from, 1));
    return list;
  });

  function onpointerdown(e: PointerEvent, task: Task) {
    if (e.button !== 0) return;
    const row = e.currentTarget as HTMLElement;
    row.setPointerCapture(e.pointerId);
    drag = { id: task.id, startY: e.clientY, dy: 0, moved: false, rowHeight: row.offsetHeight };
  }

  function onpointermove(e: PointerEvent) {
    if (!drag) return;
    drag.dy = e.clientY - drag.startY;
    if (Math.abs(drag.dy) > DRAG_THRESHOLD) drag.moved = true;
  }

  function onpointerup(task: Task) {
    if (!drag) return;
    const moved = drag.moved;
    const ids = order.map((t) => t.id);
    drag = null;
    if (moved) {
      if (ids.some((id, i) => id !== tasks.tasks[i].id)) tasks.reorder(ids);
    } else if (!task.done) {
      tasks.select(task.id);
    }
  }

  // ---- Right-click menu (native, so it is not clipped by the small window) ----
  async function oncontextmenu(e: MouseEvent, task: Task | null) {
    e.preventDefault();
    if (!isTauri()) return;
    const taskItems = task
      ? [
          { text: "Show...", action: () => onshow(task) },
          { text: "Edit...", action: () => onedit(task) },
          { text: task.done ? "Mark not done" : "Mark done", action: () => tasks.setDone(task.id, !task.done) },
          { text: "Remove", action: () => tasks.remove(task.id) },
          { item: "Separator" as const },
        ]
      : [];
    const menu = await Menu.new({
      items: [
        ...taskItems,
        { text: "Add new...", action: onadd },
        { text: "Clean all done", action: () => tasks.hideDone() },
        { text: "Copy to clipboard", action: () => navigator.clipboard.writeText(tasksAsTsv(tasks.tasks)) },
      ],
    });
    await menu.popup();
  }

  function rowClass(t: Task) {
    return {
      active: t.id === tasks.activeId,
      done: t.done,
      dragging: drag?.moved && drag.id === t.id,
      unestimated: !t.done && (t.estimated ?? 0) === 0,
      over: !t.done && t.pomodoros > (t.estimated ?? 0) && (t.estimated ?? 0) > 0,
      started: !t.done && t.pomodoros > 0,
      old: !t.done && !isToday(t.created_at),
      unplanned: t.kind === "unplanned",
    };
  }
</script>

<section class="task-list">
  <div class="grid" role="grid" aria-label="Tasks" tabindex="-1" oncontextmenu={(e) => oncontextmenu(e, null)}>
    <div class="row header" role="row">
      <span class="name" role="columnheader">Name</span>
      <span role="columnheader" title="Estimated pomodoros">E</span>
      <span role="columnheader" title="Pomodoros used">P</span>
      <span role="columnheader" title="Unplanned items that came up">U</span>
      <span role="columnheader" title="Interruptions">I</span>
    </div>

    <div class="body">
      {#each order as task (task.id)}
        <div
          class={["row", rowClass(task)]}
          role="row"
          tabindex="-1"
          aria-selected={task.id === tasks.activeId}
          title={task.kind === "unplanned" ? `Unplanned: ${task.name}` : task.name}
          onpointerdown={(e) => onpointerdown(e, task)}
          {onpointermove}
          onpointerup={() => onpointerup(task)}
          onpointercancel={() => (drag = null)}
          ondblclick={() => onedit(task)}
          oncontextmenu={(e) => {
            e.stopPropagation();
            oncontextmenu(e, task);
          }}
        >
          <span class="name" role="gridcell">{task.name}</span>
          <span role="gridcell">{task.estimated ?? ""}</span>
          <span role="gridcell">{task.pomodoros}</span>
          <span role="gridcell">{task.unplanned}</span>
          <span role="gridcell">{task.interruptions}</span>
        </div>
      {:else}
        <div class="empty">No tasks yet. Add one to get started.</div>
      {/each}
    </div>
  </div>

  <footer>
    <span class="error" role="status">{tasks.error ?? ""}</span>
    <button onclick={onadd} title="Add a task (N)">Add New</button>
  </footer>
</section>

<style>
  .task-list {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-app);
  }

  .grid {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
  }

  .body {
    flex: 1;
    overflow-y: auto;
  }

  .row {
    display: grid;
    grid-template-columns: 1fr repeat(4, 28px);
    align-items: center;
    height: 22px;
    padding: 0 4px;
    color: #cccccc;
  }

  .row > span:not(.name) {
    text-align: center;
  }

  .header {
    height: 24px;
    background: linear-gradient(#3a3a3a, #2c2c2c);
    border-bottom: 1px solid var(--border-dark);
    color: var(--text);
  }

  .header span:not(:first-child) {
    border-left: 1px solid var(--border-dark);
  }

  .body .row {
    cursor: default;
    touch-action: none;
  }

  .body .row:hover {
    background: rgba(255, 255, 255, 0.05);
  }

  .name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  /* Row styles follow the original's renderer. */
  .row.active {
    background: rgba(255, 255, 0, 0.1);
    box-shadow: inset 2px 0 0 var(--clock-work);
  }
  .row.started {
    font-weight: 700;
  }
  .row.unestimated {
    color: #777777;
  }
  .row.over {
    color: #e04040;
  }
  .row.old .name {
    font-style: italic;
  }
  .row.unplanned .name::before {
    content: "U ";
    color: var(--text-muted);
    font-weight: 400;
  }
  .row.done span {
    text-decoration: line-through;
    color: #888888;
  }
  .row.dragging {
    background: var(--bg-panel-hi);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
    position: relative;
    z-index: 1;
  }

  .empty {
    padding: 16px 8px;
    color: var(--text-muted);
    text-align: center;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px;
  }

  .error {
    flex: 1;
    min-width: 0;
    color: #e04040;
    font-size: 11px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  footer button {
    width: 91px;
    height: 24px;
  }
</style>
