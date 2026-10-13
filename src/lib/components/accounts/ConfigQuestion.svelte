<script lang="ts">
  // Renderiza uma pergunta genérica da máquina de estados do `config/create` do rclone.
  import { LoaderCircle } from "@lucide/svelte";
  import type { RcloneOption } from "../../api/types";

  let {
    option,
    error = null,
    busy = false,
    onanswer,
  }: { option: RcloneOption; error?: string | null; busy?: boolean; onanswer: (value: string) => void } = $props();

  // Provedores podem repetir itens (o OneDrive lista o mesmo drive em /me/drives e /me/drive).
  const examples = $derived.by(() => {
    const seen = new Set<string>();
    return (option.Examples ?? []).filter((ex) => !seen.has(ex.Value) && (seen.add(ex.Value), true));
  });
  const isBool = $derived(option.Type === "bool");
  const fixedChoice = $derived(option.Exclusive && examples.length > 0);

  // Começa no padrão do provedor; numa lista fechada, o padrão precisa ser um dos itens.
  function initialValue(): string {
    const def = option.DefaultStr ?? "";
    const list = option.Examples ?? [];
    if (list.length === 0 || list.some((ex) => ex.Value === def)) return def;
    return option.Exclusive || option.Required ? list[0].Value : def;
  }
  let value = $state(initialValue());

  const canAnswer = $derived(!busy && !((option.Required || fixedChoice) && !value));

  function submit() {
    if (canAnswer) onanswer(value);
  }
</script>

<div class="flex flex-col gap-4">
  <p class="selectable text-[12.5px] leading-relaxed whitespace-pre-line">{option.Help}</p>

  {#if isBool}
    <div class="flex gap-2">
      <button class="btn btn-primary" disabled={busy} onclick={() => onanswer("true")}>sim</button>
      <button class="btn" disabled={busy} onclick={() => onanswer("false")}>não</button>
      <span class="ml-2 self-center text-[11px] text-dim">padrão: {option.DefaultStr === "true" ? "sim" : "não"}</span>
    </div>
  {:else}
    {#if examples.length > 0}
      <div class="term max-h-[260px] overflow-auto py-1">
        {#each examples as ex (ex.Value)}
          <label class="flex cursor-pointer items-start gap-3 px-3 py-1.5 hover:bg-node">
            <input type="radio" bind:group={value} value={ex.Value} disabled={busy} class="mt-1 accent-[#5cb8e4]" />
            <span class="min-w-0">
              <span class="block truncate">{ex.Help || ex.Value}</span>
              {#if ex.Help && ex.Help !== ex.Value}<span class="block truncate text-[11px] text-dim">{ex.Value}</span>{/if}
            </span>
          </label>
        {/each}
      </div>
    {/if}
    {#if !fixedChoice}
      <input
        class="input"
        type={option.IsPassword ? "password" : "text"}
        bind:value
        disabled={busy}
        placeholder={option.DefaultStr || option.Name}
        onkeydown={(e) => e.key === "Enter" && submit()}
      />
    {/if}
    <div>
      <button class="btn btn-primary" disabled={!canAnswer} onclick={submit}>continuar</button>
    </div>
  {/if}

  {#if busy}
    <p class="flex items-center gap-2 text-[12px] text-muted">
      <LoaderCircle size={14} class="spin text-accent" /> consultando o provedor… (pode levar alguns segundos)
    </p>
  {/if}

  {#if error}
    <p class="selectable text-[12px] text-err">✕ {error}</p>
  {/if}
</div>
