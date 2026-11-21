<script lang="ts">
  import { onMount } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { ago, changes, sync } from "$lib/sync.svelte";

  let { keepInTray }: { keepInTray: boolean } = $props();

  // Look for nearby devices while this tab is open, and tick the countdowns.
  let now = $state(Date.now());
  onMount(() => {
    sync.discover();
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });

  const status = $derived(sync.status);
  const waitingSecs = $derived(sync.waitingUntil ? Math.max(0, Math.ceil((sync.waitingUntil - now) / 1000)) : 0);
  $effect(() => {
    if (sync.waitingUntil && waitingSecs === 0) sync.waitingUntil = null;
  });

  async function remove(device: string, name: string) {
    const ok = await ask(
      `Stop syncing with ${name}?\n\nTasks already on both devices stay where they are. You can pair again later.`,
      { title: "Remove device", kind: "warning", okLabel: "Remove", cancelLabel: "Cancel" },
    );
    if (ok) await sync.unpair(device);
  }
</script>

<div class="devices">
  <p class="intro">
    Sync your tasks with Pomodairo on your other computers over your home network. Nothing goes through
    the internet.
  </p>

  <section>
    <h3>Paired devices</h3>
    {#if status && status.peers.length}
      <ul class="list">
        {#each status.peers as p (p.device)}
          <li>
            <span class="dot" class:online={p.online} title={p.online ? "On the network now" : "Not seen on the network"}></span>
            <div class="what">
              <strong>{p.name}</strong>
              <small>
                {#if p.pending > 0}
                  <span class="warn">{changes(p.pending)} not there yet</span> ·
                {:else if p.last_sync}
                  <span class="ok">✓ Up to date</span> ·
                {/if}
                synced {ago(p.last_sync)}
              </small>
            </div>
            <button class="small" onclick={() => remove(p.device, p.name)}>Remove</button>
          </li>
        {/each}
      </ul>
      <div class="row">
        <button class="small" onclick={sync.syncNow}>Sync now</button>
        {#if !keepInTray}
          <small class="hint">Tip: turn on "Keep running in the tray" (General) so devices stay in sync while the window is closed.</small>
        {/if}
      </div>
    {:else}
      <small class="muted">No devices yet.</small>
    {/if}
  </section>

  <section>
    <h3>Add a device</h3>
    {#if sync.waitingUntil}
      <div class="waiting">
        <span>
          Waiting for another device… ({Math.floor(waitingSecs / 60)}:{String(waitingSecs % 60).padStart(2, "0")})<br />
          <small>On the other computer: Settings → Devices → choose <b>{status?.name}</b>.</small>
        </span>
        <button class="small" onclick={() => sync.cancelPairing()}>Cancel</button>
      </div>
    {:else}
      <button onclick={() => sync.openPairing()}>Pair a new device</button>
    {/if}

    <h4>Nearby devices waiting to pair</h4>
    {#if status && status.nearby.length}
      <ul class="list">
        {#each status.nearby as n (n.device)}
          <li>
            <div class="what"><strong>{n.name}</strong></div>
            <button class="small" disabled={sync.busy} onclick={() => sync.pairWith(n.device)}>
              {sync.busy ? "Pairing…" : "Pair"}
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <small class="muted">None found. On the other computer, choose "Pair a new device".</small>
    {/if}
  </section>

  {#if sync.error || status?.last_error}
    <p class="error" role="alert">{sync.error ?? status?.last_error}</p>
  {/if}
</div>

{#if sync.prompt}
  {@const prompt = sync.prompt}
  <div class="backdrop" role="presentation">
    <div class="dialog" role="dialog" aria-label="Confirm pairing code">
      <p>Pair with <strong>{prompt.peer_name}</strong>?</p>
      <div class="code" aria-label="Pairing code">{prompt.code.slice(0, 3)} {prompt.code.slice(3)}</div>
      <p class="small-print">Check that <strong>{prompt.peer_name}</strong> shows exactly the same code.</p>
      <div class="row end">
        <button onclick={() => sync.confirm(false)}>They don't match</button>
        <button class="primary" onclick={() => sync.confirm(true)}>Codes match</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .devices {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .intro {
    margin: 0;
    color: var(--text-muted);
  }

  h3 {
    margin: 0 0 6px;
    font-size: 12px;
    font-weight: 600;
  }

  h4 {
    margin: 12px 0 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
  }

  .list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
  }

  .list li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
  }

  .list li + li {
    border-top: 1px solid #2e2e2e;
  }

  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .what small {
    color: var(--text-muted);
    font-size: 11px;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #555;
    flex-shrink: 0;
  }

  .dot.online {
    background: var(--clock-break);
  }

  .ok {
    color: #7ccf8a;
  }

  .warn {
    color: #f0a020;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .row.end {
    justify-content: flex-end;
  }

  .hint,
  .muted {
    color: var(--text-muted);
    font-size: 11px;
  }

  .small {
    height: 22px;
    padding: 0 10px;
    font-size: 11px;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .waiting {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: space-between;
    padding: 8px;
    border: 1px dashed var(--border);
    border-radius: 3px;
  }

  .error {
    margin: 0;
    color: #e04040;
    font-size: 11px;
    overflow-wrap: anywhere;
  }

  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.55);
    z-index: 10;
  }

  .dialog {
    width: 320px;
    padding: 14px 16px;
    background: var(--bg-panel);
    border: 1px solid #000;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.6);
  }

  .dialog p {
    margin: 0 0 8px;
  }

  .code {
    margin: 6px 0 10px;
    text-align: center;
    font-family: var(--font-clock);
    font-weight: 700;
    font-size: 34px;
    letter-spacing: 4px;
    color: var(--clock-work);
  }

  .small-print {
    color: var(--text-muted);
    font-size: 11px;
  }

  .dialog button {
    height: 26px;
  }
</style>
