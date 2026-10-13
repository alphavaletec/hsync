<script lang="ts">
  import { toasts } from "../../stores/toasts.svelte";

  const MARK = { ok: "◆", err: "✕", info: "◇" };
  const COLOR = { ok: "text-ok", err: "text-err", info: "text-accent" };
</script>

<div class="pointer-events-none fixed right-6 bottom-12 z-50 flex w-[380px] flex-col gap-2">
  {#each toasts.items as t (t.id)}
    <button
      class="term pointer-events-auto flex items-start gap-3 px-4 py-3 text-left text-[12.5px]"
      onclick={() => toasts.dismiss(t.id)}
    >
      <span class={COLOR[t.kind]}>{MARK[t.kind]}</span>
      <span class="selectable min-w-0 break-words whitespace-pre-line">{t.text}</span>
    </button>
  {/each}
</div>
