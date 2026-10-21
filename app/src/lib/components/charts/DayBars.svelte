<script lang="ts">
  /**
   * Single-series column chart: pomodoros completed per day.
   * Marks follow the dataviz spec: <=24px columns, 4px rounded top, square at the
   * baseline, 2px gap between touching columns, hairline grid, hover tooltip.
   */
  import { shortDay, type DayCount } from "$lib/stats";
  import { niceTicks } from "./scale";

  let { data }: { data: DayCount[] } = $props();

  const HEIGHT = 170;
  const M = { top: 12, right: 8, bottom: 22, left: 28 };

  let width = $state(600);
  let hover = $state<number | null>(null);

  const plotW = $derived(Math.max(0, width - M.left - M.right));
  const plotH = HEIGHT - M.top - M.bottom;
  const ticks = $derived(niceTicks(Math.max(4, ...data.map((d) => d.pomodoros))));
  const yMax = $derived(ticks[ticks.length - 1]);
  const band = $derived(data.length ? plotW / data.length : plotW);
  const barW = $derived(Math.max(1, Math.min(24, band - 2)));
  const y = (v: number) => M.top + plotH - (v / yMax) * plotH;

  /** Column with a 4px rounded top and a square base. */
  function column(x: number, top: number, w: number, base: number): string {
    const r = Math.min(4, w / 2, base - top);
    if (r <= 0) return "";
    return `M${x},${base}V${top + r}Q${x},${top} ${x + r},${top}H${x + w - r}Q${x + w},${top} ${x + w},${top + r}V${base}Z`;
  }

  // Label roughly every 50px so dates never collide.
  const labelEvery = $derived(Math.max(1, Math.ceil(56 / Math.max(band, 1))));

  const tip = $derived(hover === null ? null : data[hover]);
</script>

<figure class="chart" bind:clientWidth={width}>
  <svg width={width} height={HEIGHT} role="img" aria-label="Pomodoros completed per day">
    {#each ticks as t (t)}
      <line class="grid" x1={M.left} x2={width - M.right} y1={y(t)} y2={y(t)} />
      <text class="tick" x={M.left - 6} y={y(t)} dy="0.32em" text-anchor="end">{t}</text>
    {/each}

    {#each data as d, i (d.day)}
      {@const x = M.left + i * band + (band - barW) / 2}
      {#if d.pomodoros > 0}
        <path class="bar" class:dim={hover !== null && hover !== i} d={column(x, y(d.pomodoros), barW, y(0))} />
      {/if}
      {#if i % labelEvery === 0}
        <text class="tick" x={M.left + i * band + band / 2} y={HEIGHT - 6} text-anchor="middle">
          {d.day.slice(5).replace("-", "/")}
        </text>
      {/if}
      <!-- Hit target: the whole column band, taller than the mark. -->
      <rect
        class="hit"
        x={M.left + i * band}
        y={M.top}
        width={band}
        height={plotH}
        role="presentation"
        onpointerenter={() => (hover = i)}
        onpointerleave={() => (hover = null)}
      />
    {/each}
    <line class="axis" x1={M.left} x2={width - M.right} y1={y(0)} y2={y(0)} />
  </svg>

  {#if tip && hover !== null}
    {@const left = Math.min(Math.max(M.left + hover * band + band / 2, 70), width - 70)}
    <div class="tooltip" style:left="{left}px">
      <strong>{shortDay(tip.day)}</strong>
      <span>{tip.pomodoros} pomodoro{tip.pomodoros === 1 ? "" : "s"}</span>
      <span class="muted">{tip.interruptions} interruptions · {tip.unplanned} unplanned</span>
      <span class="muted">{tip.tasks_done} task{tip.tasks_done === 1 ? "" : "s"} done</span>
    </div>
  {/if}
</figure>

<style>
  .chart {
    position: relative;
    margin: 0;
  }

  svg {
    display: block;
    overflow: visible;
  }

  .grid {
    stroke: var(--chart-grid);
    stroke-width: 1;
  }

  .axis {
    stroke: var(--chart-axis);
    stroke-width: 1;
  }

  .tick {
    fill: var(--text-muted);
    font-size: 10px;
  }

  .bar {
    fill: var(--chart-mark);
    transition: opacity 0.1s;
  }

  .bar.dim {
    opacity: 0.45;
  }

  .hit {
    fill: transparent;
  }

  .tooltip {
    position: absolute;
    top: 0;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px 8px;
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: 3px;
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
  }

  .muted {
    color: var(--text-muted);
  }
</style>
