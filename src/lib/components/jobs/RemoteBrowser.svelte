<script lang="ts">
  import { ArrowUp, Folder, FolderPlus, LoaderCircle } from "@lucide/svelte";
  import { api, errorMessage } from "../../api/commands";
  import type { DirEntry } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import Modal from "../ui/Modal.svelte";

  let {
    remote,
    initialPath = "",
    onpick,
    onclose,
  }: { remote: string; initialPath?: string; onpick: (path: string) => void; onclose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let path = $state(initialPath.replace(/^\/+|\/+$/g, ""));
  let dirs = $state<DirEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let newName = $state("");

  async function load() {
    loading = true;
    error = null;
    try {
      dirs = await api.listRemoteDirs(remote, path);
    } catch (e) {
      error = errorMessage(e);
      dirs = [];
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void path;
    void load();
  });

  const crumbs = $derived(path ? path.split("/") : []);

  async function mkdir() {
    const name = newName.trim();
    if (!name) return;
    const target = path ? `${path}/${name}` : name;
    const ok = await app.act(() => api.makeRemoteDir(remote, target));
    if (ok !== undefined) {
      newName = "";
      path = target;
    }
  }
</script>

<Modal title="{remote}: — escolher pasta" width="560px" {onclose}>
  <div class="mb-3 flex flex-wrap items-center gap-1 text-[12.5px]">
    <button class="text-accent hover:underline" onclick={() => (path = "")}>{remote}:</button>
    {#each crumbs as c, i (i)}
      <span class="text-dim">/</span>
      <button class="hover:underline" onclick={() => (path = crumbs.slice(0, i + 1).join("/"))}>{c}</button>
    {/each}
  </div>

  <div class="term h-[300px] overflow-auto py-1">
    {#if path}
      <button
        class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-muted hover:bg-node"
        onclick={() => (path = crumbs.slice(0, -1).join("/"))}
      >
        <ArrowUp size={14} /> ..
      </button>
    {/if}
    {#if loading}
      <div class="flex items-center gap-2 px-3 py-2 text-muted"><LoaderCircle size={14} class="spin" /> listando…</div>
    {:else if error}
      <div class="selectable px-3 py-2 text-err">✕ {error}</div>
    {:else if dirs.length === 0}
      <div class="px-3 py-2 text-dim">◇ nenhuma subpasta</div>
    {:else}
      {#each dirs as d (d.path)}
        <button class="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-node" onclick={() => (path = d.path)}>
          <Folder size={14} class="text-accent" />
          <span class="truncate">{d.name}</span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="mt-3 flex gap-2">
    <input class="input" placeholder="nova pasta aqui" bind:value={newName} onkeydown={(e) => e.key === "Enter" && mkdir()} />
    <button class="btn" onclick={mkdir} disabled={!newName.trim()}><FolderPlus size={14} /> criar</button>
  </div>

  {#snippet footer()}
    <span class="mr-auto truncate text-[12px] text-muted">{remote}:{path || "/"}</span>
    <button class="btn" onclick={onclose}>cancelar</button>
    <button class="btn btn-primary" onclick={() => onpick(path)}>usar esta pasta</button>
  {/snippet}
</Modal>
