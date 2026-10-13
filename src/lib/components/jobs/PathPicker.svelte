<script lang="ts">
  import { Cloud, FolderOpen, HardDrive } from "@lucide/svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { app } from "../../stores/app.svelte";
  import { providerLabel, remoteOf } from "../../utils/format";
  import RemoteBrowser from "./RemoteBrowser.svelte";

  let { value = $bindable(""), label }: { value: string; label: string } = $props();

  // Estado inicial derivado do valor recebido; depois o usuário controla os campos.
  const initialRemote = remoteOf(value);
  let kind = $state<"local" | "remote">(initialRemote || (!value && app.remotes.length) ? "remote" : "local");
  let remote = $state(initialRemote ?? app.remotes[0]?.name ?? "");
  let remotePath = $state(initialRemote ? value.slice(initialRemote.length + 1) : "");
  let localPath = $state(initialRemote ? "" : value);
  let browsing = $state(false);

  $effect(() => {
    value = kind === "local" ? localPath.trim() : remote ? `${remote}:${remotePath.trim().replace(/^\/+/, "")}` : "";
  });

  async function pickLocal() {
    const dir = await open({ directory: true, multiple: false, title: `Escolher ${label.toLowerCase()}` });
    if (typeof dir === "string") localPath = dir;
  }
</script>

<div>
  <div class="mb-1.5 flex items-center justify-between">
    <span class="field-label mb-0">{label}</span>
    <div class="flex text-[11.5px]">
      <button
        class="flex items-center gap-1 border border-line px-2 py-0.5 {kind === 'local' ? 'bg-node text-fg' : 'text-muted'}"
        onclick={() => (kind = "local")}
      >
        <HardDrive size={12} /> este PC
      </button>
      <button
        class="-ml-px flex items-center gap-1 border border-line px-2 py-0.5 {kind === 'remote' ? 'bg-node text-fg' : 'text-muted'}"
        onclick={() => (kind = "remote")}
        disabled={app.remotes.length === 0}
        title={app.remotes.length === 0 ? "Conecte uma conta primeiro" : ""}
      >
        <Cloud size={12} /> conta
      </button>
    </div>
  </div>

  {#if kind === "local"}
    <div class="flex gap-2">
      <input class="input" bind:value={localPath} placeholder="C:\Users\voce\Documentos" />
      <button class="btn" onclick={pickLocal}><FolderOpen size={14} /> procurar</button>
    </div>
  {:else}
    <div class="flex gap-2">
      <select class="input w-[180px] shrink-0" bind:value={remote}>
        {#each app.remotes as r (r.name)}
          <option value={r.name}>{r.name} ({providerLabel(r.kind)})</option>
        {/each}
      </select>
      <input class="input" bind:value={remotePath} placeholder="pasta/no/remoto (vazio = raiz)" />
      <button class="btn" onclick={() => (browsing = true)} disabled={!remote}><FolderOpen size={14} /> navegar</button>
    </div>
  {/if}
</div>

{#if browsing}
  <RemoteBrowser
    {remote}
    initialPath={remotePath}
    onclose={() => (browsing = false)}
    onpick={(p) => {
      remotePath = p;
      browsing = false;
    }}
  />
{/if}
