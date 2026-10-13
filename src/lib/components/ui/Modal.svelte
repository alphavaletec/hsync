<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    width = "640px",
    onclose,
    children,
    footer,
  }: {
    title: string;
    width?: string;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="fixed inset-0 z-40 flex items-center justify-center bg-black/60 p-6" role="presentation">
  <div
    class="term flex max-h-full w-full flex-col"
    style:max-width={width}
    role="dialog"
    aria-modal="true"
    aria-label={title}
  >
    <div class="term-bar shrink-0">
      <span class="dots"><i></i><i></i><i></i></span>
      <span class="mx-auto text-fg">{title}</span>
      <button class="btn-ghost btn btn-icon h-6 w-6" onclick={onclose} aria-label="Fechar">✕</button>
    </div>
    <div class="min-h-0 flex-1 overflow-auto p-5">
      {@render children()}
    </div>
    {#if footer}
      <div class="flex shrink-0 items-center justify-end gap-2 border-t border-line px-5 py-3">
        {@render footer()}
      </div>
    {/if}
  </div>
</div>
