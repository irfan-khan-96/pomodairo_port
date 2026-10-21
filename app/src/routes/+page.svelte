<script lang="ts">
  import { onMount } from "svelte";
  import Clock, { type ClockState } from "$lib/components/Clock.svelte";
  import MiniView from "$lib/components/MiniView.svelte";
  import TaskDetails from "$lib/components/TaskDetails.svelte";
  import TaskInput, { type InputMode } from "$lib/components/TaskInput.svelte";
  import TaskList from "$lib/components/TaskList.svelte";
  import TitleBar from "$lib/components/TitleBar.svelte";
  import { shortDescription, tasks, type Task } from "$lib/tasks.svelte";
  import { formatClock, timer } from "$lib/timer.svelte";
  import { view } from "$lib/view.svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { settings, ticketNumber } from "$lib/settings.svelte";

  onMount(() => {
    const unlisten = Promise.all([timer.connect(), tasks.connect(), view.connect(), settings.connect()]);
    return () => void unlisten.then((fns) => fns.forEach((fn) => fn()));
  });

  // ---- Timer display ----
  const snap = $derived(timer.snapshot);
  const active = $derived(tasks.active);

  const clockState: ClockState = $derived(
    snap.phase === "break" ? "break" : snap.phase === "time_out" || snap.phase === "break_over" ? "timeout" : "work",
  );

  const progress = $derived.by(() => {
    switch (snap.phase) {
      case "pomodoro":
      case "break":
        return snap.total_secs > 0 ? 1 - snap.remaining_secs / snap.total_secs : 0;
      case "time_out":
      case "break_over":
        return 1;
      default:
        return 0;
    }
  });

  const running = $derived(snap.phase !== "idle" && snap.phase !== "break_over");
  const inPomodoro = $derived(snap.phase === "pomodoro");

  // Without a task the primary button creates one, as in the original.
  const primaryLabel = $derived(
    inPomodoro ? "Stop Timer" : running ? "Stop Break" : active ? "Start Timer" : "New Task",
  );

  const taskLabel = $derived.by(() => {
    switch (snap.phase) {
      case "time_out":
        return "Time's up!";
      case "break":
        return snap.break_kind === "long" ? "On Long Break" : "On Break";
      case "break_over":
        return "Break over" + (active ? ` · next: ${active.name}` : "");
      default:
        return active ? shortDescription(active) : "No task selected";
    }
  });

  // ---- Views: mini (clock only), short (clock + buttons), full (with task list) ----
  const listOpen = $derived(view.view === "full");

  function setListOpen(open: boolean) {
    view.setView(open ? "full" : "short");
  }

  // ---- Dialogs ----
  let input = $state<{ mode: InputMode; task?: Task } | null>(null);
  let details = $state<Task | null>(null);

  function openInput(mode: InputMode, task?: Task) {
    // Mini view is too small for dialogs, as in the original.
    if (view.view === "mini") return;
    if ((mode === "unplanned" || mode === "interruption") && !active) return;
    input = { mode, task };
  }

  async function submitInput(name: string, estimated: number | null) {
    const current = input;
    input = null;
    if (!current) return;
    switch (current.mode) {
      case "create":
        await tasks.create(name, estimated);
        break;
      case "edit":
        if (current.task) await tasks.update(current.task.id, name, estimated);
        break;
      case "unplanned":
      case "interruption":
        await tasks.logDisturbance(current.mode, name, estimated);
        break;
    }
  }

  // ---- "URL to open": clicking a task named like "Fix #123" opens <url>123 ----
  const ticketUrl = $derived.by(() => {
    const base = settings.current.general.url_to_open;
    const ticket = active ? ticketNumber(active.name) : null;
    return base && ticket ? base + ticket : null;
  });

  // ---- Actions ----
  function onPrimary() {
    if (running) timer.stop();
    else if (!active) openInput("create");
    else {
      timer.start();
      // The original closed the task list when a pomodoro started.
      if (listOpen) setListOpen(false);
    }
  }

  function onDone() {
    if (active) tasks.setDone(active.id, true);
  }

  // Shortcuts: Space start/stop, N new task, U unplanned, I interruption, M mini view.
  // Keyup is swallowed too, so a focused button is not also activated by Space.
  const isTyping = (e: KeyboardEvent) => e.target instanceof HTMLInputElement;
  const dialogOpen = $derived(input !== null || details !== null);

  function onKeydown(e: KeyboardEvent) {
    if (isTyping(e) || dialogOpen || e.ctrlKey || e.metaKey || e.altKey) return;
    const action = {
      Space: onPrimary,
      KeyN: () => openInput("create"),
      KeyU: () => openInput("unplanned"),
      KeyI: () => openInput("interruption"),
      KeyM: view.toggleMini,
    }[e.code];
    if (!action) return;
    e.preventDefault();
    if (!e.repeat) action();
  }

  function onKeyup(e: KeyboardEvent) {
    if (e.code === "Space" && !isTyping(e)) e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} onkeyup={onKeyup} />

<main class="app">
  {#if view.view === "mini"}
    <MiniView text={formatClock(snap.remaining_secs)} label={taskLabel} state={clockState} {progress} />
  {:else}
    <section class="timer-panel" data-tauri-drag-region>
      <TitleBar />

      <div class="timer-body" data-tauri-drag-region>
        <div class="readout" data-tauri-drag-region>
          <Clock text={formatClock(snap.remaining_secs)} state={clockState} {progress} />
          {#if ticketUrl && snap.phase !== "break" && snap.phase !== "time_out"}
            <button class="task-label link" title="Open {ticketUrl}" onclick={() => openUrl(ticketUrl!)}>
              {taskLabel}
            </button>
          {:else}
            <div class="task-label" title={active?.name ?? ""}>{taskLabel}</div>
          {/if}
        </div>

        <div class="actions">
          <button class="primary" title="Start/stop (Space)" onclick={onPrimary}>{primaryLabel}</button>
          <button class="secondary" title="Mark the active task done" disabled={!active || inPomodoro} onclick={onDone}>
            Done
          </button>
        </div>
      </div>

      <footer class="control-bar">
        <button title="Open/close task list" onclick={() => setListOpen(!listOpen)}>Task List</button>
        <button title="Add unplanned (U)" disabled={!active} onclick={() => openInput("unplanned")}>Unplanned</button>
        <button title="Add interruption (I)" disabled={!active} onclick={() => openInput("interruption")}>
          Interruption
        </button>
      </footer>
    </section>

    {#if listOpen}
      <TaskList
        onadd={() => openInput("create")}
        onedit={(task) => openInput("edit", task)}
        onshow={(task) => (details = task)}
      />
    {/if}
  {/if}
</main>

{#if input}
  <TaskInput
    mode={input.mode}
    initialName={input.task?.name}
    initialEstimate={input.task?.estimated}
    onsubmit={submitInput}
    oncancel={() => (input = null)}
  />
{/if}

{#if details}
  <TaskDetails task={details} onclose={() => (details = null)} />
{/if}

<style>
  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border-dark);
  }

  .timer-panel {
    display: flex;
    flex-direction: column;
    height: 165px;
    flex-shrink: 0;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border-dark);
  }

  .timer-body {
    flex: 1;
    display: flex;
    gap: 8px;
    padding: 6px 8px 0 10px;
  }

  .readout {
    flex: 1;
    min-width: 0;
  }

  .task-label {
    margin-top: 6px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  button.task-label.link {
    display: block;
    max-width: 100%;
    padding: 0;
    background: none;
    border: none;
    box-shadow: none;
    text-align: left;
    color: #9cc8ff;
    cursor: pointer;
  }

  button.task-label.link:hover {
    text-decoration: underline;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 90px;
    flex-shrink: 0;
  }

  .primary {
    height: 50px;
  }

  .secondary {
    height: 25px;
  }

  .control-bar {
    display: flex;
    gap: 10px;
    padding: 8px 12px;
    background: var(--bg-panel-hi);
    border-top: 1px solid var(--border-dark);
  }

  .control-bar button {
    height: 22px;
  }
</style>
