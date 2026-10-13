<script lang="ts">
  import { formatSpeed } from "../../utils/format";

  let { samples, height = 64 }: { samples: number[]; height?: number } = $props();

  const W = 300;
  const max = $derived(Math.max(1, ...samples));
  const points = $derived(
    samples.map((v, i) => {
      const x = samples.length <= 1 ? W : (i / (samples.length - 1)) * W;
      const y = height - 2 - (v / max) * (height - 6);
      return [x, y] as const;
    }),
  );
  const line = $derived(points.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" "));
  const area = $derived(points.length ? `${line} L${W},${height} L0,${height} Z` : "");
  const current = $derived(samples.at(-1) ?? 0);
</script>

<div class="relative">
  <svg viewBox="0 0 {W} {height}" preserveAspectRatio="none" class="block w-full" style:height="{height}px">
    {#each [0.25, 0.5, 0.75] as f (f)}
      <line x1="0" x2={W} y1={height * f} y2={height * f} stroke="#25292e" vector-effect="non-scaling-stroke" />
    {/each}
    {#if points.length > 1}
      <path d={area} fill="rgb(92 184 228 / 0.10)" />
      <path d={line} fill="none" stroke="#5cb8e4" stroke-width="1.5" vector-effect="non-scaling-stroke" />
    {/if}
  </svg>
  <div class="mt-1 flex justify-between text-[11px] text-dim">
    <span>agora: <span class="text-accent">{formatSpeed(current)}</span></span>
    <span>pico: {formatSpeed(max === 1 ? 0 : max)}</span>
  </div>
</div>
