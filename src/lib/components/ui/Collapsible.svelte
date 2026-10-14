<script lang="ts">
  // Bloco que começa recolhido: o cabeçalho (título + resumo) abre e fecha o conteúdo.
  import { ChevronRight } from "@lucide/svelte";
  import type { Snippet } from "svelte";

  let {
    id,
    title,
    summary,
    children,
  }: { id: string; title: string; summary?: Snippet; children: Snippet } = $props();

  // Lembra a escolha neste computador (opcional: sem armazenamento, começa recolhido).
  const storageKey = () => `hsync.open.${id}`;
  function read(): boolean {
    try {
      return localStorage.getItem(storageKey()) === "1";
    } catch {
      return false;
    }
  }
  let open = $state(read());

  function toggle() {
    open = !open;
    try {
      localStorage.setItem(storageKey(), open ? "1" : "0");
    } catch {
      /* sem armazenamento: vale só nesta sessão */
    }
  }
</script>

<section class="border-t border-line-soft">
  <button
    class="flex w-full items-center gap-2 px-5 py-3 text-left hover:bg-node/50"
    aria-expanded={open}
    aria-controls="blk-{id}"
    onclick={toggle}
  >
    <ChevronRight size={14} class="shrink-0 text-muted transition-transform {open ? 'rotate-90' : ''}" />
    <span class="sec-label">{title}</span>
    {#if summary}
      <span class="ml-2 min-w-0 flex-1 truncate text-[11.5px] text-dim">{@render summary()}</span>
    {/if}
    <span class="ml-auto shrink-0 text-[11px] text-dim">{open ? "recolher" : "expandir"}</span>
  </button>
  {#if open}
    <div id="blk-{id}" class="px-5 pb-4">
      {@render children()}
    </div>
  {/if}
</section>
