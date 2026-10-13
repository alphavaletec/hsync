<script lang="ts">
  import { tick } from "svelte";
  import { app } from "../../stores/app.svelte";

  let onlyProblems = $state(false);
  let box: HTMLDivElement | undefined = $state();
  let follow = $state(true);

  const LEVEL_COLOR: Record<string, string> = {
    error: "text-err",
    critical: "text-err",
    warning: "text-warn",
    notice: "text-accent",
  };

  const lines = $derived(
    (onlyProblems ? app.logs.filter((l) => ["error", "critical", "warning"].includes(l.level)) : app.logs).slice(-300),
  );

  $effect(() => {
    void lines.length;
    if (follow && box) tick().then(() => box && (box.scrollTop = box.scrollHeight));
  });

  function onscroll() {
    if (!box) return;
    follow = box.scrollHeight - box.scrollTop - box.clientHeight < 24;
  }

  function time(t: string) {
    const d = new Date(t);
    return isNaN(d.getTime()) ? "" : d.toLocaleTimeString("pt-BR");
  }
</script>

<div class="term">
  <div class="term-bar">
    <span class="dots"><i></i><i></i><i></i></span>
    <span class="mx-auto">rclone — log</span>
    <label class="flex cursor-pointer items-center gap-1.5 text-[11px]">
      <input type="checkbox" bind:checked={onlyProblems} class="accent-[#5cb8e4]" /> só erros
    </label>
  </div>
  <div bind:this={box} {onscroll} class="selectable h-[180px] overflow-auto px-4 py-2 text-[11.5px] leading-[1.7]">
    {#if lines.length === 0}
      <span class="text-dim">{onlyProblems ? "nenhum erro registrado." : "aguardando saída do rclone…"}</span>
    {/if}
    {#each lines as l, i (i)}
      <div class="flex gap-3">
        <span class="shrink-0 text-dim">{time(l.time)}</span>
        <span class="w-[52px] shrink-0 {LEVEL_COLOR[l.level] ?? 'text-muted'}">{l.level}</span>
        <span class="min-w-0 break-all whitespace-pre-wrap text-fg/85">{l.msg}</span>
      </div>
    {/each}
  </div>
</div>
