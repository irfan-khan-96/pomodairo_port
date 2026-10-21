<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { isTauri } from "@tauri-apps/api/core";
  import DayBars from "$lib/components/charts/DayBars.svelte";
  import FactorLine from "$lib/components/charts/FactorLine.svelte";
  import {
    fetchCompleted,
    fetchDisturbances,
    fetchHashtags,
    fetchPerDay,
    fetchReality,
    lastDays,
    localDateTime,
    shortDay,
    toTsv,
    type CompletedTask,
    type DateRange,
    type DayCount,
    type Disturbance,
    type Hashtags,
    type WeekFactor,
  } from "$lib/stats";

  type Tab = "pomodoros" | "interruptions" | "count" | "reality";
  const TABS: { id: Tab; label: string }[] = [
    { id: "pomodoros", label: "Pomodoros" },
    { id: "interruptions", label: "Interruptions" },
    { id: "count", label: "Count" },
    { id: "reality", label: "Reality factors" },
  ];
  const PRESETS = [
    { label: "Today", days: 1 },
    { label: "7 days", days: 7 },
    { label: "30 days", days: 30 },
  ];

  let tab = $state<Tab>("pomodoros");
  let range = $state<DateRange>(lastDays(7));
  // Each filtered tab keeps its own filter (their tag lists differ).
  let filters = $state({ pomodoros: "", interruptions: "" });
  let error = $state<string | null>(null);
  let copied = $state(false);

  let completed = $state<CompletedTask[]>([]);
  let disturbances = $state<Disturbance[]>([]);
  let perDay = $state<DayCount[]>([]);
  let reality = $state<WeekFactor[]>([]);
  let tags = $state<Hashtags>({ tasks: [], disturbances: [] });

  const usesRange = $derived(tab !== "reality");

  async function refresh() {
    error = null;
    try {
      const r = $state.snapshot(range);
      switch (tab) {
        case "pomodoros":
          completed = await fetchCompleted(r, filters.pomodoros);
          break;
        case "interruptions":
          disturbances = await fetchDisturbances(r, filters.interruptions);
          break;
        case "count":
          perDay = await fetchPerDay(r);
          break;
        case "reality":
          reality = await fetchReality();
          break;
      }
    } catch (e) {
      error = String(e);
    }
  }

  // Re-query whenever the tab, range or filter changes.
  $effect(() => {
    void tab;
    void range.from;
    void range.to;
    void filters.pomodoros;
    void filters.interruptions;
    refresh();
  });

  onMount(() => {
    fetchHashtags().then((t) => (tags = t));
    if (!isTauri()) return;
    // Keep figures live while the window is open.
    const unlisten = listen("tasks://changed", () => {
      refresh();
      fetchHashtags().then((t) => (tags = t));
    });
    return () => void unlisten.then((fn) => fn());
  });

  const totals = $derived({
    pomodoros: perDay.reduce((s, d) => s + d.pomodoros, 0),
    interruptions: perDay.reduce((s, d) => s + d.interruptions + d.unplanned, 0),
    tasksDone: perDay.reduce((s, d) => s + d.tasks_done, 0),
  });

  function tableTsv(): string {
    switch (tab) {
      case "pomodoros":
        return toTsv(
          ["Name", "Type", "Closed", "Estimated", "Pomodoros", "Delta", "Interruptions"],
          completed.map((t) => [t.name, t.kind, localDateTime(t.closed_at), t.estimated, t.pomodoros, t.delta, t.interruptions]),
        );
      case "interruptions":
        return toTsv(
          ["Name", "Type", "Created", "Task"],
          disturbances.map((d) => [d.name, d.kind, localDateTime(d.created_at), d.task]),
        );
      case "count":
        return toTsv(
          ["Day", "Pomodoros", "Interruptions", "Unplanned", "Tasks done"],
          perDay.map((d) => [d.day, d.pomodoros, d.interruptions, d.unplanned, d.tasks_done]),
        );
      case "reality":
        return toTsv(
          ["Week", "Week start", "Reality factor", "Estimated", "Pomodoros", "Delta", "Interruptions"],
          reality.map((w) => [w.week, w.week_start, w.factor, w.estimated, w.pomodoros, w.delta, w.interruptions]),
        );
    }
  }

  async function copy() {
    await navigator.clipboard.writeText(tableTsv());
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  // Numbers and signed deltas align right; dates and names stay left.
  const isNum = (v: unknown) => typeof v === "number" || /^[+-]?\d+(\.\d+)?$/.test(String(v));

  const signed = (n: number | null) => (n === null ? "" : n > 0 ? `+${n}` : String(n));
</script>

<main class="stats">
  <div class="tabs" role="tablist">
    {#each TABS as t (t.id)}
      <button role="tab" aria-selected={tab === t.id} class:active={tab === t.id} onclick={() => (tab = t.id)}>
        {t.label}
      </button>
    {/each}
  </div>

  <!-- Filters in one row above the content. -->
  <div class="toolbar">
    {#if usesRange}
      <input type="date" aria-label="From" bind:value={range.from} max={range.to} />
      <span class="muted">to</span>
      <input type="date" aria-label="To" bind:value={range.to} min={range.from} />
      {#each PRESETS as p (p.days)}
        <button class="chip" onclick={() => (range = lastDays(p.days))}>{p.label}</button>
      {/each}
    {/if}
    {#if tab === "pomodoros" || tab === "interruptions"}
      <input
        class="filter"
        type="search"
        placeholder="Filter or #tag"
        list="tag-list"
        bind:value={filters[tab]}
        aria-label="Filter by name"
      />
      <datalist id="tag-list">
        {#each tab === "pomodoros" ? tags.tasks : tags.disturbances as tag (tag)}
          <option value={tag}></option>
        {/each}
      </datalist>
    {/if}
    <span class="spacer"></span>
    <button onclick={copy} title="Copy the table (tab-separated)">{copied ? "Copied" : "Copy"}</button>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <section class="content">
    {#if tab === "pomodoros"}
      {@render table(
        ["Name", "Type", "Closed", "E", "P", "Δ", "I"],
        completed.map((t) => [t.name, t.kind === "unplanned" ? "U" : "P", localDateTime(t.closed_at), t.estimated ?? "", t.pomodoros, signed(t.delta), t.interruptions]),
        "No tasks were finished in this period.",
      )}
    {:else if tab === "interruptions"}
      {@render table(
        ["Name", "Type", "Created", "Task"],
        disturbances.map((d) => [d.name, d.kind === "unplanned" ? "Unplanned" : "Interruption", localDateTime(d.created_at), d.task ?? "—"]),
        "No interruptions in this period.",
      )}
    {:else if tab === "count"}
      <div class="summary">
        <h2>Pomodoros completed per day</h2>
        <span class="muted">
          {totals.pomodoros} pomodoros · {totals.interruptions} interruptions · {totals.tasksDone} tasks done
        </span>
      </div>
      <DayBars data={perDay} />
      {@render table(
        ["Day", "Pomodoros", "Interruptions", "Unplanned", "Tasks done"],
        [...perDay].reverse().map((d) => [shortDay(d.day), d.pomodoros, d.interruptions, d.unplanned, d.tasks_done]),
        "No days in this period.",
      )}
    {:else}
      <div class="summary">
        <h2>Weekly reality factor</h2>
        <span class="muted">Pomodoros used per pomodoro estimated. Above 1.0 means tasks took longer than planned.</span>
      </div>
      {#if reality.some((w) => w.factor !== null)}
        <FactorLine data={reality} />
      {/if}
      {@render table(
        ["Week", "Starts", "Factor", "Estimated", "Pomodoros", "Δ", "Interruptions"],
        [...reality].reverse().map((w) => [w.week, w.week_start, w.factor ?? "—", w.estimated, w.pomodoros, signed(w.delta), w.interruptions]),
        "No estimated tasks yet.",
      )}
    {/if}
  </section>
</main>

{#snippet table(headers: string[], rows: (string | number)[][], empty: string)}
  <table>
    <thead>
      <tr>
        {#each headers as h, i (i)}<th class:num={i > 0 && rows.length > 0 && isNum(rows[0][i])}>{h}</th>{/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row, r (r)}
        <tr>
          {#each row as cell, i (i)}<td class:num={i > 0 && isNum(cell)}>{cell}</td>{/each}
        </tr>
      {:else}
        <tr><td class="empty" colspan={headers.length}>{empty}</td></tr>
      {/each}
    </tbody>
  </table>
{/snippet}

<style>
  :global(body) {
    background: var(--bg-panel);
  }

  .stats {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }

  .tabs {
    display: flex;
    gap: 2px;
    padding: 8px 10px 0;
    background: var(--bg-app);
    border-bottom: 1px solid var(--border-dark);
  }

  .tabs button {
    border-radius: 4px 4px 0 0;
    border-bottom: none;
    padding: 4px 14px;
    background: var(--bg-app);
    box-shadow: none;
    color: var(--text-muted);
  }

  .tabs button.active {
    background: var(--bg-panel);
    color: var(--text);
    border-color: var(--border-dark);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-dark);
  }

  .toolbar input {
    height: 24px;
    padding: 0 6px;
    font: inherit;
    color: var(--text);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: 2px;
    color-scheme: dark;
  }

  input[type="date"] {
    width: 112px;
  }

  .filter {
    flex: 1;
    min-width: 90px;
    max-width: 180px;
    user-select: text;
    -webkit-user-select: text;
  }

  .toolbar button {
    height: 24px;
  }

  .chip {
    padding: 0 8px;
    font-size: 11px;
  }

  .spacer {
    flex: 1;
  }

  .muted {
    color: var(--text-muted);
  }

  .error {
    margin: 8px 12px 0;
    color: #e04040;
  }

  .content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .summary h2 {
    margin: 0 0 2px;
    font-size: 13px;
    font-weight: 600;
  }

  .summary .muted {
    font-size: 11px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }

  th {
    position: sticky;
    top: -10px;
    text-align: left;
    font-weight: 600;
    padding: 4px 6px;
    background: linear-gradient(#3a3a3a, #2c2c2c);
    border-bottom: 1px solid var(--border-dark);
  }

  td {
    padding: 3px 6px;
    border-bottom: 1px solid #2e2e2e;
    color: #cccccc;
    user-select: text;
    -webkit-user-select: text;
  }

  .num {
    text-align: right;
  }

  tbody tr:hover td {
    background: rgba(255, 255, 255, 0.04);
  }

  .empty {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
  }
</style>
