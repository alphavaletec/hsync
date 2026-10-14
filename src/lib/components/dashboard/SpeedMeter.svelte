<script lang="ts">
  // Velocidade atual e pico, em uma linha, com um gráfico pequeno (fica na barra do topo).
  import { app } from "../../stores/app.svelte";
  import { formatSpeed } from "../../utils/format";

  const W = 72;
  const H = 16;
  const samples = $derived(app.speedHistory);
  const max = $derived(Math.max(1, ...samples));
  const current = $derived(samples.at(-1) ?? 0);
  const line = $derived(
    samples
      .map((v, i) => {
        const x = samples.length <= 1 ? W : (i / (samples.length - 1)) * W;
        const y = H - 1 - (v / max) * (H - 3);
        return `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" "),
  );
</script>

<span class="flex items-center gap-2 whitespace-nowrap" title="velocidade de transferência (últimas amostras)">
  <svg viewBox="0 0 {W} {H}" width={W} height={H} class="hidden shrink-0 sm:block" aria-hidden="true">
    <line x1="0" x2={W} y1={H - 0.5} y2={H - 0.5} stroke="#25292e" />
    {#if samples.length > 1}
      <path d={line} fill="none" stroke="#5cb8e4" stroke-width="1.25" />
    {/if}
  </svg>
  <span class="text-dim">agora <span class={current > 0 ? "text-accent" : "text-muted"}>{formatSpeed(current)}</span></span>
  <span class="hidden text-dim md:inline">pico {formatSpeed(max === 1 ? 0 : max)}</span>
</span>
