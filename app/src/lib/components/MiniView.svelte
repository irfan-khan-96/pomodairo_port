<script lang="ts">
  import type { ClockState } from "./Clock.svelte";
  import TitleBar from "./TitleBar.svelte";

  /** The original's mini view: a progress bar labelled with the task, and a small clock. */
  let {
    text,
    label,
    state,
    progress,
  }: { text: string; label: string; state: ClockState; progress: number } = $props();
</script>

<section class="mini {state}" data-tauri-drag-region>
  <TitleBar compact />
  <div class="row" data-tauri-drag-region>
    <div class="bar" title={label} data-tauri-drag-region>
      <div class="fill" style:width="{Math.min(Math.max(progress, 0), 1) * 100}%"></div>
      <span class="label" data-tauri-drag-region>{label}</span>
    </div>
    <span class="clock" data-tauri-drag-region>{text}</span>
  </div>
</section>

<style>
  .mini {
    --clock-color: var(--clock-work);
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
  }
  .mini.break {
    --clock-color: var(--clock-break);
  }
  .mini.timeout {
    --clock-color: var(--clock-timeout);
  }

  .row {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px 2px;
  }

  .bar {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 20px;
    background: var(--bg-sunken);
    border: 1px solid var(--border-dark);
    border-radius: 2px;
    overflow: hidden;
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: color-mix(in srgb, var(--clock-color) 30%, transparent);
    transition: width 0.3s linear;
  }

  .label {
    position: relative;
    display: block;
    padding: 0 6px;
    line-height: 18px;
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: center;
  }

  .clock {
    font-family: var(--font-clock);
    font-weight: 700;
    font-size: 18px;
    color: var(--clock-color);
    text-shadow: 0 0 6px color-mix(in srgb, var(--clock-color) 55%, transparent);
  }

  .timeout .clock {
    animation: blink 0.6s steps(1) infinite;
  }

  @keyframes blink {
    50% {
      opacity: 0.15;
    }
  }
</style>
