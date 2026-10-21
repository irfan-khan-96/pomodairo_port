<script lang="ts">
  /**
   * Single-series line: weekly reality factor (pomodoros used ÷ estimated), with a
   * reference line at 1.0. 2px line, 8px markers with a 2px surface ring, crosshair
   * + tooltip on hover, the latest value labelled at the end.
   */
  import type { WeekFactor } from "$lib/stats";
  import { niceTicks } from "./scale";

  let { data }: { data: WeekFactor[] } = $props();

  const HEIGHT = 170;
  const M = { top: 14, right: 40, bottom: 22, left: 32 };

  let width = $state(600);
  let hover = $state<number | null>(null);

  const points = $derived(data.filter((d) => d.factor !== null));
  const plotW = $derived(Math.max(0, width - M.left - M.right));
  const plotH = HEIGHT - M.top - M.bottom;
  const ticks = $derived(niceTicks(Math.max(1.5, ...points.map((d) => d.factor!))));
  const yMax = $derived(ticks[ticks.length - 1]);
  const x = (i: number) => M.left + (points.length > 1 ? (i / (points.length - 1)) * plotW : plotW / 2);
  const y = (v: number) => M.top + plotH - (v / yMax) * plotH;
  const path = $derived(points.map((d, i) => `${i ? "L" : "M"}${x(i)},${y(d.factor!)}`).join(""));
  const labelEvery = $derived(Math.max(1, Math.ceil(64 / Math.max(plotW / Math.max(points.length, 1), 1))));

  function onmove(e: PointerEvent) {
    if (!points.length) return;
    const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
    const px = e.clientX - rect.left;
    const step = points.length > 1 ? plotW / (points.length - 1) : plotW;
    hover = Math.max(0, Math.min(points.length - 1, Math.round((px - M.left) / (step || 1))));
  }

  const tip = $derived(hover === null ? null : points[hover]);
  const last = $derived(points[points.length - 1]);
</script>

<figure class="chart" bind:clientWidth={width}>
  <svg
    width={width}
    height={HEIGHT}
    role="img"
    aria-label="Weekly reality factor"
    onpointermove={onmove}
    onpointerleave={() => (hover = null)}
  >
    {#each ticks as t (t)}
      <line class="grid" x1={M.left} x2={width - M.right} y1={y(t)} y2={y(t)} />
      <text class="tick" x={M.left - 6} y={y(t)} dy="0.32em" text-anchor="end">{t}</text>
    {/each}

    <line class="reference" x1={M.left} x2={width - M.right} y1={y(1)} y2={y(1)} />
    <text class="tick" x={width - M.right + 4} y={y(1)} dy="0.32em">on est.</text>

    {#each points as d, i (d.week)}
      {#if i % labelEvery === 0}
        <text class="tick" x={x(i)} y={HEIGHT - 6} text-anchor="middle">{d.week.slice(5)}</text>
      {/if}
    {/each}

    {#if tip && hover !== null}
      <line class="crosshair" x1={x(hover)} x2={x(hover)} y1={M.top} y2={M.top + plotH} />
    {/if}

    <path class="line" d={path} />
    {#each points as d, i (d.week)}
      <circle class="dot" class:active={hover === i} cx={x(i)} cy={y(d.factor!)} r={hover === i ? 5 : 4} />
    {/each}

    {#if last && hover === null}
      <!-- Above the last point, so it never collides with the reference label in the margin. -->
      <text class="end-label" x={x(points.length - 1)} y={y(last.factor!) - 10} text-anchor="middle">{last.factor}</text>
    {/if}
  </svg>

  {#if tip && hover !== null}
    {@const left = Math.min(Math.max(x(hover), 80), width - 80)}
    <div class="tooltip" style:left="{left}px">
      <strong>Week {tip.week.slice(6)} · from {tip.week_start}</strong>
      <span>Reality factor {tip.factor}</span>
      <span class="muted">{tip.pomodoros} used / {tip.estimated} estimated</span>
      <span class="muted">{tip.interruptions} interruptions</span>
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

  .reference {
    stroke: var(--chart-axis);
    stroke-width: 1;
  }

  .crosshair {
    stroke: var(--chart-axis);
    stroke-width: 1;
  }

  .tick {
    fill: var(--text-muted);
    font-size: 10px;
  }

  .end-label {
    fill: var(--text);
    font-size: 11px;
    font-weight: 600;
  }

  .line {
    fill: none;
    stroke: var(--chart-mark);
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .dot {
    fill: var(--chart-mark);
    stroke: var(--chart-surface);
    stroke-width: 2;
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
