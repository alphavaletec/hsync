<script lang="ts">
  let {
    values = $bindable([]),
    placeholder = "",
    prefix = "",
  }: { values: string[]; placeholder?: string; prefix?: string } = $props();

  let draft = $state("");

  function commit() {
    const items = draft
      .split(/[,;\n]/)
      .map((s) => s.trim())
      .filter(Boolean);
    const next = [...values];
    for (const it of items) if (!next.includes(it)) next.push(it);
    values = next;
    draft = "";
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      commit();
    } else if (e.key === "Backspace" && !draft && values.length) {
      values = values.slice(0, -1);
    }
  }
</script>

<div class="input flex h-auto min-h-[32px] flex-wrap items-center gap-1.5 py-1">
  {#each values as v (v)}
    <span class="chip">
      {prefix}{v}
      <button class="text-dim hover:text-err" onclick={() => (values = values.filter((x) => x !== v))} aria-label="Remover {v}">✕</button>
    </span>
  {/each}
  <input
    class="min-w-[120px] flex-1 bg-transparent text-[12.5px] outline-none"
    bind:value={draft}
    {onkeydown}
    onblur={commit}
    placeholder={values.length ? "" : placeholder}
  />
</div>
