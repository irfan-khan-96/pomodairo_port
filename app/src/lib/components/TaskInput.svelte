<script module lang="ts">
  export type InputMode = "create" | "edit" | "unplanned" | "interruption";
</script>

<script lang="ts">
  /**
   * Inline input overlay, like the original's popup: a name field plus an
   * estimate field (hidden for interruptions). Enter saves, Escape cancels.
   */

  let {
    mode,
    initialName = "",
    initialEstimate = null,
    onsubmit,
    oncancel,
  }: {
    mode: InputMode;
    initialName?: string;
    initialEstimate?: number | null;
    onsubmit: (name: string, estimated: number | null) => void;
    oncancel: () => void;
  } = $props();

  // Seeded once from the props; the dialog is recreated for each use.
  // svelte-ignore state_referenced_locally
  let name = $state(initialName);
  // svelte-ignore state_referenced_locally
  let estimate = $state(initialEstimate == null ? "" : String(initialEstimate));
  let focused = $state<"name" | "estimate">("name");

  const showEstimate = $derived(mode !== "interruption");
  const title = $derived(
    { create: "New task", edit: "Edit task", unplanned: "Unplanned", interruption: "Interruption" }[mode],
  );
  const hint = $derived(focused === "name" ? "description" : "estimate (0–7)");

  function submit() {
    const trimmed = name.trim();
    if (!trimmed) return oncancel();
    onsubmit(trimmed, showEstimate && estimate !== "" ? Number(estimate) : null);
  }

  function onkeydown(e: KeyboardEvent) {
    e.stopPropagation(); // keep app shortcuts (Space etc.) out of the text fields
    if (e.key === "Enter") submit();
    else if (e.key === "Escape") oncancel();
  }

  // Like the original, the estimate is a single digit 0–7.
  function onEstimateInput(e: Event) {
    const el = e.currentTarget as HTMLInputElement;
    estimate = el.value.replace(/[^0-7]/g, "").slice(-1);
    el.value = estimate;
  }

  function autofocus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }
</script>

<div class="backdrop" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && oncancel()}>
  <div class="dialog" role="dialog" aria-label={title}>
    <div class="hint">{title}: enter {hint} (Esc to cancel)</div>
    <div class="fields">
      <input
        class="name"
        type="text"
        maxlength="200"
        aria-label="Description"
        bind:value={name}
        {onkeydown}
        onfocus={() => (focused = "name")}
        use:autofocus
      />
      {#if showEstimate}
        <input
          class="estimate"
          type="text"
          inputmode="numeric"
          aria-label="Estimate"
          value={estimate}
          oninput={onEstimateInput}
          {onkeydown}
          onfocus={() => (focused = "estimate")}
        />
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: start center;
    padding-top: 52px;
    background: rgba(0, 0, 0, 0.45);
    z-index: 10;
  }

  .dialog {
    width: 280px;
    padding: 4px 10px 10px;
    background: #181818;
    border: 1px solid #000;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.6);
    animation: fade-in 0.15s ease-out;
  }

  .hint {
    color: #8a8a8a;
    font-size: 11px;
    padding-bottom: 3px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .fields {
    display: flex;
    gap: 6px;
  }

  input {
    height: 28px;
    font: inherit;
    font-size: 14px;
    color: #fff;
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: 2px;
    padding: 0 6px;
    user-select: text;
    -webkit-user-select: text;
  }

  input:focus {
    outline: none;
    border-color: var(--text-muted);
  }

  .name {
    flex: 1;
    min-width: 0;
  }

  .estimate {
    width: 36px;
    text-align: center;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
</style>
