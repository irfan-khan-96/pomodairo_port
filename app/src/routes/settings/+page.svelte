<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import {
    DEFAULT_SETTINGS,
    TABS,
    settings,
    type DatabaseLocation,
    type Settings,
    type SettingsTab,
  } from "$lib/settings.svelte";
  import { view } from "$lib/view.svelte";

  const LABELS: Record<SettingsTab, string> = {
    general: "General",
    timing: "Timing",
    sounds: "Sounds",
    advanced: "Advanced",
  };

  const initialTab = new URLSearchParams(location.search).get("tab") as SettingsTab | null;
  let tab = $state<SettingsTab>(initialTab && TABS.includes(initialTab) ? initialTab : "general");

  // Editable copies; nothing is applied until Save.
  let draft = $state<Settings>(structuredClone(DEFAULT_SETTINGS));
  let alwaysOnTop = $state(false);
  let rememberPosition = $state(true);
  let db = $state<DatabaseLocation | null>(null);
  let error = $state<string | null>(null);
  let saving = $state(false);

  onMount(() => {
    const cleanups: Promise<() => void>[] = [settings.connect(), view.connect()];
    (async () => {
      await Promise.all(cleanups);
      draft = structuredClone($state.snapshot(settings.current));
      alwaysOnTop = view.prefs.always_on_top;
      rememberPosition = view.prefs.remember_position;
      if (isTauri()) db = await invoke<DatabaseLocation>("database_location");
    })();
    if (isTauri()) {
      // The main window asks for a tab when the settings window is already open.
      cleanups.push(getCurrentWebviewWindow().listen<SettingsTab>("settings://tab", (e) => (tab = e.payload)));
    }
    return () => cleanups.forEach((c) => c.then((fn) => fn()));
  });

  async function close() {
    if (isTauri()) await getCurrentWebviewWindow().close();
  }

  async function save() {
    saving = true;
    error = null;
    try {
      await settings.save($state.snapshot(draft));
      if (alwaysOnTop !== view.prefs.always_on_top) await view.setAlwaysOnTop(alwaysOnTop);
      if (rememberPosition !== view.prefs.remember_position) await view.setRememberPosition(rememberPosition);
      await close();
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  // ---- Sounds ----
  type SoundKind = "alarm" | "ticking";
  const fileKey = (kind: SoundKind): "alarm_file" | "ticking_file" => (kind === "alarm" ? "alarm_file" : "ticking_file");

  async function browseSound(kind: SoundKind) {
    const picked = await open({
      title: kind === "alarm" ? "Choose alarm sound" : "Choose ticking sound",
      filters: [{ name: "Audio", extensions: ["mp3", "wav", "ogg"] }],
    });
    if (typeof picked === "string") draft.sound[fileKey(kind)] = picked;
  }

  async function preview(kind: SoundKind) {
    error = null;
    try {
      await invoke("sound_preview", { kind, file: draft.sound[fileKey(kind)], volume: draft.sound.volume });
    } catch (e) {
      error = String(e);
    }
  }

  // ---- Database location ----
  async function moveDatabase(dir: string | null) {
    const where = dir ?? "the default folder";
    const ok = await ask(
      `Use the database in ${where}?\n\nIf there is no Pomodairo database there yet, the current one is copied. Pomodairo will restart.`,
      { title: "Change database location", kind: "warning", okLabel: "Restart", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    try {
      await invoke("database_move", { dir });
    } catch (e) {
      error = String(e);
    }
  }

  async function browseDatabaseFolder() {
    const picked = await open({ directory: true, title: "Choose database folder" });
    if (typeof picked === "string") await moveDatabase(picked);
  }

  const usingDefault = $derived(!!db && db.path.startsWith(db.default_dir));

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
    else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) save();
  }
</script>

<svelte:window {onkeydown} />

<main class="settings">
  <div class="tabs" role="tablist">
    {#each TABS as t (t)}
      <button role="tab" aria-selected={tab === t} class:active={tab === t} onclick={() => (tab = t)}>
        {LABELS[t]}
      </button>
    {/each}
  </div>

  <section class="panel" role="tabpanel">
    {#if tab === "general"}
      <label class="check"><input type="checkbox" bind:checked={alwaysOnTop} /> Always on top</label>
      <label class="check">
        <input type="checkbox" bind:checked={draft.general.update_taskbar} />
        Show remaining time in the taskbar title
      </label>
      <label class="check"><input type="checkbox" bind:checked={rememberPosition} /> Remember window position</label>
      <label class="check">
        <input type="checkbox" bind:checked={draft.general.notifications} />
        Show a notification when a pomodoro or break ends
      </label>
      <label class="field">
        <span>URL to open (e.g. your bug tracker)</span>
        <input type="url" placeholder="https://tracker.example.com/issue/" bind:value={draft.general.url_to_open} />
        <small>Clicking the task name opens this URL with the task's #number appended.</small>
      </label>
    {:else if tab === "timing"}
      {@render slider("Pomodoro length", "min", 1, 60, draft.timing.pomodoro_minutes, (v) => (draft.timing.pomodoro_minutes = v))}
      {@render slider("Short break length", "min", 1, 30, draft.timing.short_break_minutes, (v) => (draft.timing.short_break_minutes = v))}
      {@render slider("Long break length", "min", 1, 60, draft.timing.long_break_minutes, (v) => (draft.timing.long_break_minutes = v))}
      {@render slider("Long break every", "pomodoros", 1, 30, draft.timing.long_break_interval, (v) => (draft.timing.long_break_interval = v))}
      <small>Changes apply from the next pomodoro or break.</small>
    {:else if tab === "sounds"}
      <label class="check"><input type="checkbox" bind:checked={draft.sound.enabled} /> Sound enabled</label>
      {@render slider("Volume", "%", 0, 100, draft.sound.volume, (v) => (draft.sound.volume = v), 5)}
      <label class="check">
        <input type="checkbox" bind:checked={draft.sound.fade_ticking} /> Fade the ticking out after a few seconds
      </label>
      {@render soundFile("alarm", "Alarm sound")}
      {@render soundFile("ticking", "Ticking sound")}
    {:else}
      <div class="field">
        <span>Database location</span>
        <input type="text" readonly value={db?.path ?? "…"} title={db?.path} />
        {#if db?.overridden}
          <small>Set by the POMODAIRO_DB environment variable.</small>
        {:else}
          <div class="row">
            <button onclick={browseDatabaseFolder}>Change folder…</button>
            <button disabled={usingDefault} onclick={() => moveDatabase(null)}>Use default</button>
          </div>
          <small>Point this at a synced folder to share your tasks between computers.</small>
        {/if}
      </div>
    {/if}
  </section>

  <footer>
    <span class="error" role="alert">{error ?? ""}</span>
    <button onclick={close}>Cancel</button>
    <button class="primary" disabled={saving} onclick={save}>Save</button>
  </footer>
</main>

{#snippet slider(label: string, unit: string, min: number, max: number, value: number, set: (v: number) => void, step = 1)}
  <label class="field">
    <span>{label} <b>{value} {unit}</b></span>
    <input type="range" {min} {max} {step} {value} oninput={(e) => set(Number(e.currentTarget.value))} />
  </label>
{/snippet}

{#snippet soundFile(kind: SoundKind, label: string)}
  {@const key = fileKey(kind)}
  <div class="field">
    <span>{label}</span>
    <div class="row">
      <input type="text" placeholder="Built-in sound" bind:value={draft.sound[key]} title={draft.sound[key] ?? ""} />
      <button onclick={() => browseSound(kind)}>Browse…</button>
      <button class="icon" title="Play" aria-label="Play {label.toLowerCase()}" onclick={() => preview(kind)}>▶</button>
      <button
        class="icon"
        title="Use built-in sound"
        aria-label="Use built-in {label.toLowerCase()}"
        disabled={!draft.sound[key]}
        onclick={() => (draft.sound[key] = null)}>✕</button
      >
    </div>
  </div>
{/snippet}

<style>
  :global(body) {
    background: var(--bg-panel);
  }

  .settings {
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

  .panel {
    flex: 1;
    overflow-y: auto;
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field > span {
    color: var(--text-muted);
  }

  .field b {
    color: var(--text);
    font-weight: 600;
    margin-left: 4px;
  }

  small {
    color: var(--text-muted);
    font-size: 11px;
  }

  .row {
    display: flex;
    gap: 6px;
  }

  .row input {
    flex: 1;
    min-width: 0;
  }

  input[type="text"],
  input[type="url"] {
    height: 24px;
    padding: 0 6px;
    font: inherit;
    color: var(--text);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: 2px;
    user-select: text;
    -webkit-user-select: text;
  }

  input:focus-visible {
    outline: 1px solid var(--text-muted);
  }

  input[readonly] {
    color: var(--text-muted);
  }

  input[type="range"],
  input[type="checkbox"] {
    accent-color: var(--clock-work);
  }

  .icon {
    width: 26px;
    padding: 0;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid var(--border-dark);
    background: var(--bg-app);
  }

  footer button {
    width: 72px;
    height: 24px;
  }

  .error {
    flex: 1;
    min-width: 0;
    color: #e04040;
    font-size: 11px;
    overflow-wrap: anywhere;
  }
</style>
