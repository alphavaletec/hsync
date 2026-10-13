<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    num,
    crumb,
    title,
    subtitle,
    legend,
    actions,
  }: {
    num: string;
    crumb: string;
    title: string;
    subtitle?: string;
    legend?: Snippet;
    actions?: Snippet;
  } = $props();
</script>

<div class="flex h-9 items-center border-b border-line-soft px-5 text-[12px]">
  <span class="text-accent">{num}</span>
  <span class="ml-3 text-muted">/ {crumb}</span>
  <div class="ml-auto flex items-center gap-4 text-muted">{@render legend?.()}</div>
</div>

<div class="flex items-end gap-6 px-5 pt-5 pb-4">
  <div class="min-w-0">
    <h1 class="truncate text-[26px] leading-tight font-bold tracking-tight text-fg">{title}</h1>
    {#if subtitle}
      <p class="truncate text-[22px] leading-tight font-bold tracking-tight text-accent">{subtitle}</p>
    {/if}
  </div>
  <div class="ml-auto flex shrink-0 items-center gap-2 pb-1">{@render actions?.()}</div>
</div>
