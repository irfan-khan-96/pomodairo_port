<script module lang="ts">
  export type ClockState = "work" | "break" | "timeout";
</script>

<script lang="ts">
  let {
    text,
    state = "work",
    progress = 0,
  }: { text: string; state?: ClockState; progress?: number } = $props();
</script>

<div class="clock {state}">
  <div class="digits">
    <!-- Unlit segments behind the digits, like a real LCD -->
    <span class="ghost" aria-hidden="true">88:88</span>
    <span class="lit">{text}</span>
  </div>
  <div class="progress" role="progressbar" aria-valuemin="0" aria-valuemax="1" aria-valuenow={progress}>
    <div class="fill" style:width="{Math.min(Math.max(progress, 0), 1) * 100}%"></div>
  </div>
</div>

<style>
  .clock {
    --clock-color: var(--clock-work);
  }
  .clock.break {
    --clock-color: var(--clock-break);
  }
  .clock.timeout {
    --clock-color: var(--clock-timeout);
  }

  .digits {
    position: relative;
    font-family: var(--font-clock);
    font-weight: 700;
    font-size: 50px;
    line-height: 1;
    letter-spacing: 1px;
  }

  .ghost {
    position: absolute;
    inset: 0;
    color: var(--clock-color);
    opacity: 0.07;
  }

  .lit {
    position: relative;
    color: var(--clock-color);
    text-shadow: 0 0 8px color-mix(in srgb, var(--clock-color) 55%, transparent);
  }

  .timeout .lit {
    animation: blink 0.6s steps(1) infinite;
  }

  @keyframes blink {
    50% {
      opacity: 0.15;
    }
  }

  .progress {
    margin-top: 8px;
    height: 3px;
    background: var(--bg-sunken);
    border-radius: 2px;
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--clock-color);
    opacity: 0.8;
    transition: width 0.3s linear;
  }
</style>
