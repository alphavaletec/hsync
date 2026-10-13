<script lang="ts">
  // Diagrama em nós (estilo circuito da referência): motor rclone no topo,
  // ligado à origem e ao destino.
  import { Cloud, HardDrive } from "@lucide/svelte";
  import type { SyncMode } from "../../api/types";
  import { MODE_LABEL, directionOf, isLocalPath, remoteOf } from "../../utils/format";
  import HMark from "../ui/HMark.svelte";

  let {
    source,
    dest,
    mode,
    active = false,
  }: { source: string; dest: string; mode: SyncMode; active?: boolean } = $props();

  function short(p: string) {
    const r = remoteOf(p);
    const path = r ? p.slice(r.length + 1) || "/" : p;
    const parts = path.split(/[\\/]/).filter(Boolean);
    return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : path;
  }
  const arrow = $derived(mode === "bisync" ? "⇄" : "→");
</script>

<div class="relative select-none">
  <!-- motor -->
  <div class="node mx-auto flex w-[190px] items-center gap-3 px-4 py-3">
    <span class="node-dot"></span>
    <HMark size={22} class="text-fg" />
    <div class="min-w-0">
      <div class="text-[14px] font-semibold">rclone</div>
      <div class="truncate text-[11px] text-muted">{MODE_LABEL[mode]} · {directionOf(source, dest)}</div>
    </div>
  </div>

  <!-- conectores -->
  <svg viewBox="0 0 300 70" class="block h-[70px] w-full" preserveAspectRatio="none" aria-hidden="true">
    <g fill="none" stroke={active ? "#5cb8e4" : "#3b4248"} stroke-width="2" vector-effect="non-scaling-stroke">
      <path d="M150 0 V22 L140 32 H82 L72 42 V70" class:flow={active} vector-effect="non-scaling-stroke" />
      <path d="M150 0 V22 L160 32 H218 L228 42 V70" class:flow={active} vector-effect="non-scaling-stroke" />
    </g>
    <rect x="146" y="0" width="8" height="6" fill={active ? "#5cb8e4" : "#4a5057"} />
  </svg>
  <div class="absolute top-[78px] left-1/2 -translate-x-1/2 bg-frame px-2 text-[12px] {active ? 'text-accent' : 'text-dim'}">
    {arrow}
  </div>

  <!-- origem / destino -->
  <div class="grid grid-cols-2 gap-5 px-2">
    {#each [{ label: "origem", path: source }, { label: "destino", path: dest }] as end (end.label)}
      <div class="node min-w-0 px-3 py-3">
        <span class="node-dot"></span>
        <div class="flex items-center gap-2">
          {#if isLocalPath(end.path)}
            <HardDrive size={16} class="shrink-0 text-muted" />
          {:else}
            <Cloud size={16} class="shrink-0 text-accent" />
          {/if}
          <span class="truncate text-[12.5px] font-semibold">{remoteOf(end.path) ?? "este PC"}</span>
        </div>
        <div class="mt-1 truncate text-[11px] text-muted" title={end.path}>{short(end.path)}</div>
        <div class="mt-1 text-[10.5px] text-dim">{end.label}</div>
      </div>
    {/each}
  </div>
</div>

<style>
  .flow {
    stroke-dasharray: 6 5;
    animation: flow 0.9s linear infinite;
  }
  @keyframes flow {
    to {
      stroke-dashoffset: -11;
    }
  }
</style>
