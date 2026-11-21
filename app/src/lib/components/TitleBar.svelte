<script lang="ts">
  import { isTauri } from "@tauri-apps/api/core";
  import { LogicalPosition } from "@tauri-apps/api/dpi";
  import { Menu } from "@tauri-apps/api/menu";
  import { openSettings } from "$lib/settings.svelte";
  import { changes, sync } from "$lib/sync.svelte";
  import { openStats } from "$lib/stats";
  import { view } from "$lib/view.svelte";
  import { closeWindow, minimizeWindow } from "$lib/window";

  let { compact = false }: { compact?: boolean } = $props();

  // Native menu, so it is not clipped by the small window.
  async function openMenu(e: MouseEvent) {
    if (!isTauri()) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const p = view.prefs;
    const menu = await Menu.new({
      items: [
        {
          text: "Settings",
          items: [
            { text: "General...", action: () => openSettings("general") },
            { text: "Timing...", action: () => openSettings("timing") },
            { text: "Sounds...", action: () => openSettings("sounds") },
            { text: "Devices...", action: () => openSettings("devices") },
            { text: "Advanced...", action: () => openSettings("advanced") },
          ],
        },
        { text: "Statistics...", action: openStats },
        { item: "Separator" },
        { text: "Always on top", checked: p.always_on_top, action: () => view.setAlwaysOnTop(!p.always_on_top) },
        {
          text: "Remember window position",
          checked: p.remember_position,
          action: () => view.setRememberPosition(!p.remember_position),
        },
        { item: "Separator" },
        { text: "Mini view (M)", action: () => view.toggleMini() },
      ],
    });
    await menu.popup(new LogicalPosition(rect.left, rect.bottom));
  }
</script>

<header class="titlebar" class:compact data-tauri-drag-region>
  <!-- As in the original, the menu is unavailable in mini view. -->
  <button class="menu" title="Menu" disabled={compact} onclick={openMenu}>Menu</button>
  <div class="spacer" data-tauri-drag-region></div>
  {#if sync.status && sync.status.peers.length && !compact}
    {@const waiting = sync.status.peers.filter((p) => p.pending > 0)}
    <!-- Hand-off indicator: is everything on the other devices yet? -->
    <button
      class="sync"
      class:pending={waiting.length > 0}
      title={waiting.length
        ? `${changes(sync.status.pending_total)} not yet on ${waiting.map((p) => p.name).join(", ")}`
        : `Up to date on ${sync.status.peers.map((p) => p.name).join(", ")}`}
      onclick={() => openSettings("devices")}
    >
      {waiting.length ? `↑${sync.status.pending_total}` : "✓"}
    </button>
  {/if}
  <button class="view" title="Toggle mini view (M)" onclick={view.toggleMini}>view</button>
  <button class="icon" title="Minimise" aria-label="Minimise" onclick={minimizeWindow}>
    <svg viewBox="0 0 10 10" width="10" height="10"><path d="M1 8h8" stroke="currentColor" stroke-width="1.5" /></svg>
  </button>
  <button class="icon" title="Close" aria-label="Close" onclick={closeWindow}>
    <svg viewBox="0 0 10 10" width="10" height="10">
      <path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" stroke-width="1.5" />
    </svg>
  </button>
</header>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 22px;
    flex-shrink: 0;
    padding: 0 4px 0 0;
  }

  .spacer {
    flex: 1;
    align-self: stretch;
  }

  .menu {
    height: 22px;
    padding: 0 16px;
    border-radius: 0 0 4px 0;
    border-top: none;
    border-left: none;
  }

  .sync {
    height: 18px;
    min-width: 22px;
    padding: 0 6px;
    font-size: 11px;
    color: #7ccf8a;
  }

  .sync.pending {
    color: #f0a020;
  }

  .view {
    height: 18px;
    padding: 0 10px;
    font-size: 11px;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 20px;
    height: 18px;
    padding: 0;
    background: none;
    border: none;
    box-shadow: none;
    color: var(--text);
  }

  .icon:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
  }

  .icon:last-child:hover {
    background: #c42b1c;
  }
</style>
