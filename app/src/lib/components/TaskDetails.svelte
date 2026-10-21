<script lang="ts">
  import type { Task } from "$lib/tasks.svelte";

  let { task, onclose }: { task: Task; onclose: () => void } = $props();

  const fmt = (iso: string | null) => (iso ? new Date(iso).toLocaleString() : "—");
</script>

<svelte:window onkeydown={(e) => (e.key === "Escape" || e.key === "Enter") && onclose()} />

<div class="backdrop" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-label="Task details">
    <h2>{task.name}</h2>
    <dl>
      <dt>Type</dt>
      <dd>{task.kind === "unplanned" ? "Unplanned" : "Task"}{task.done ? " (done)" : ""}</dd>
      <dt>Estimated</dt>
      <dd>{task.estimated ?? "—"}</dd>
      <dt>Worked</dt>
      <dd>{task.pomodoros}</dd>
      <dt>Unplanned</dt>
      <dd>{task.unplanned}</dd>
      <dt>Interruptions</dt>
      <dd>{task.interruptions}</dd>
      <dt>Created</dt>
      <dd>{fmt(task.created_at)}</dd>
      <dt>Closed</dt>
      <dd>{fmt(task.closed_at)}</dd>
    </dl>
    <button onclick={onclose}>OK</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 10;
  }

  .dialog {
    width: 280px;
    padding: 10px 12px 12px;
    background: var(--bg-panel);
    border: 1px solid #000;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.6);
    display: flex;
    flex-direction: column;
  }

  h2 {
    margin: 0 0 8px;
    font-size: 13px;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 12px;
    margin: 0 0 10px;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
  }

  button {
    align-self: flex-end;
    width: 70px;
    height: 22px;
  }
</style>
