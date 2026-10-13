<script lang="ts">
  // Aba Arquivos: navega qualquer conta mesclando a nuvem com a cópia local e permite
  // escolher, por arquivo/pasta, o que sincroniza, fica offline, só sobe ou é ignorado.
  import {
    ArrowUp,
    Cloud,
    Download,
    Eye,
    File,
    Folder,
    FolderOpen,
    FolderPlus,
    HardDrive,
    LoaderCircle,
    RefreshCw,
    Upload,
  } from "@lucide/svelte";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { onDestroy } from "svelte";
  import { api, errorMessage } from "../../api/commands";
  import { on } from "../../api/events";
  import type { FileEntry, FilesConfig, ItemState, OnDemandStatus } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ITEM_STATE, STATUS_COLOR, STATUS_LABEL, formatBytes, formatDateTime, formatInterval, providerLabel } from "../../utils/format";
  import PreviewModal from "../files/PreviewModal.svelte";
  import ViewHeader from "../ui/ViewHeader.svelte";

  const STATES: ItemState[] = ["sync", "download", "upload", "ignore"];
  const INTERVALS = [0, 15, 30, 60, 180, 360, 1440];

  let remote = $state<string | null>(app.filesRemote ?? app.remotes[0]?.name ?? null);
  let path = $state("");
  let entries = $state<FileEntry[]>([]);
  let config = $state<FilesConfig | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let selected = $state<Set<string>>(new Set());
  let busyPath = $state<string | null>(null);
  let newFolder = $state<string | null>(null);
  /** Índice em `fileEntries` do arquivo em pré-visualização. */
  let previewIndex = $state<number | null>(null);

  const crumbs = $derived(path ? path.split("/") : []);
  const selectedEntries = $derived(entries.filter((e) => selected.has(e.path)));
  const fileEntries = $derived(entries.filter((e) => !e.isDir));
  /** Alguma tarefa desta conta está rodando agora? (itens marcados aparecem como "baixando"). */
  const accountSyncing = $derived(app.progress.some((p) => managedIds.has(p.jobId)));
  const keepsLocal = (s: ItemState | undefined) => s === "sync" || s === "download";

  function openPreview(e: FileEntry) {
    const i = fileEntries.indexOf(e);
    if (i >= 0) previewIndex = i;
  }
  const managedIds = $derived(new Set(config?.jobs.map((j) => j.id) ?? []));
  const counts = $derived.by(() => {
    const c: Record<ItemState, number> = { sync: 0, download: 0, upload: 0, ignore: 0 };
    for (const s of config?.selections ?? []) c[s.state]++;
    return c;
  });

  // As contas podem chegar depois da aba abrir, ou a conta escolhida pode ter sido removida.
  $effect(() => {
    const names = app.remotes.map((r) => r.name);
    if (names.length > 0 && (remote === null || !names.includes(remote))) {
      remote = names[0] ?? null;
      path = "";
    }
  });

  $effect(() => {
    app.filesRemote = remote;
  });

  // Recarrega ao trocar de conta ou pasta.
  $effect(() => {
    const r = remote;
    const p = path;
    if (r) void load(r, p);
  });

  $effect(() => {
    const r = remote;
    if (r) api.filesConfig(r).then((c) => (config = c)).catch((e) => toasts.push("err", errorMessage(e)));
  });

  // Arquivos sob demanda (estilo OneDrive) desta conta.
  let ondemand = $state<OnDemandStatus | null>(null);
  let ondemandBusy = $state(false);
  $effect(() => {
    const r = remote;
    ondemand = null;
    if (r) api.ondemandStatus(r).then((s) => (ondemand = s)).catch(() => {});
  });

  async function enableOndemand() {
    if (!remote) return;
    ondemandBusy = true;
    try {
      ondemand = await api.enableOndemand(remote);
      toasts.push("ok", `Arquivos sob demanda ativados: abra "HSync - ${remote}" no Explorer`, 7000);
      if (ondemand.path) await revealItemInDir(ondemand.path).catch(() => {});
    } catch (e) {
      toasts.push("err", errorMessage(e));
    } finally {
      ondemandBusy = false;
    }
  }

  async function disableOndemand() {
    if (!remote) return;
    const ok = await ask(
      `Desativar os arquivos sob demanda de "${remote}"? A pasta sai do Explorer e vai para a Lixeira — os arquivos continuam na nuvem.`,
      { title: "Arquivos sob demanda", kind: "warning", okLabel: "Desativar", cancelLabel: "Cancelar" },
    );
    if (!ok) return;
    ondemandBusy = true;
    try {
      await api.disableOndemand(remote);
      ondemand = await api.ondemandStatus(remote);
    } catch (e) {
      toasts.push("err", errorMessage(e));
    } finally {
      ondemandBusy = false;
    }
  }

  // Depois que uma sincronização desta conta (ou transferência avulsa) termina, atualiza a coluna "onde".
  let unlisten: (() => void) | undefined;
  on("job-finished", (f) => {
    if (remote && (f.jobId < 0 || managedIds.has(f.jobId))) {
      void load(remote, path, true);
      api.filesConfig(remote).then((c) => (config = c));
    }
  }).then((u) => (unlisten = u));
  onDestroy(() => unlisten?.());

  async function load(r: string, p: string, quiet = false) {
    if (!quiet) loading = true;
    error = null;
    try {
      const list = await api.browse(r, p);
      if (r === remote && p === path) {
        entries = list;
        selected = new Set();
      }
    } catch (e) {
      error = errorMessage(e);
      entries = [];
    } finally {
      loading = false;
    }
  }

  /** Estado efetivo: o da própria entrada ou o herdado do ancestral mais próximo. */
  function effective(p: string): { state: ItemState; explicit: boolean; from?: string } | null {
    let best: { state: ItemState; path: string } | null = null;
    for (const s of config?.selections ?? []) {
      if (s.path === p) return { state: s.state, explicit: true };
      if (s.isDir && p.startsWith(s.path + "/") && (!best || s.path.length > best.path.length)) {
        best = { state: s.state, path: s.path };
      }
    }
    return best ? { state: best.state, explicit: false, from: best.path } : null;
  }

  async function setState(e: FileEntry, value: string) {
    if (!remote) return;
    const st = (value || null) as ItemState | null;
    busyPath = e.path;
    try {
      const r = remote;
      const res = await api.setItemState(r, e.path, e.isDir, st);
      config = res.config;
      if (res.notice) toasts.push(res.notice.startsWith("Mantive") ? "info" : "ok", res.notice, 9000);
      else if (st === "download" || st === "sync") toasts.push("ok", `${e.name}: ${ITEM_STATE[st].label} em ${config.base}`);
      // Reflete na hora a coluna "onde" (cópia local liberada ou mantida).
      await load(r, path, true);
    } catch (err) {
      toasts.push("err", errorMessage(err));
    } finally {
      busyPath = null;
    }
  }

  function toggle(p: string) {
    const next = new Set(selected);
    if (next.has(p)) next.delete(p);
    else next.add(p);
    selected = next;
  }

  /** Selecionados com cópia local liberável: estão nos dois lados e não são ⇄/↓/↑. */
  const freeable = $derived(
    selectedEntries.filter((e) => e.inCloud && e.local && !keepsLocal(effective(e.path)?.state) && effective(e.path)?.state !== "upload"),
  );
  let freeing = $state(false);

  async function freeSelected() {
    if (!remote || freeable.length === 0) return;
    const ok = await ask(
      `Liberar o espaço de ${freeable.length} item(ns)? A cópia local vai para a Lixeira; os arquivos continuam na nuvem. Nada que exista só neste PC é removido.`,
      { title: "Liberar espaço", kind: "warning", okLabel: "Liberar", cancelLabel: "Cancelar" },
    );
    if (!ok) return;
    const r = remote;
    freeing = true;
    try {
      const notices = await api.freeSpace(r, freeable.map((e) => ({ path: e.path, isDir: e.isDir })));
      for (const n of notices) toasts.push(n.startsWith("Cópia local") ? "ok" : "info", n, 9000);
      await load(r, path, true);
    } catch (err) {
      toasts.push("err", errorMessage(err));
    } finally {
      freeing = false;
    }
  }

  async function downloadSelected() {
    if (!remote || selectedEntries.length === 0) return;
    const target = await open({ directory: true, title: "Baixar para…" });
    if (typeof target !== "string") return;
    const items = selectedEntries.filter((e) => e.inCloud).map((e) => ({ path: e.path, isDir: e.isDir }));
    const ok = await app.act(() => api.downloadItems(remote!, items, target));
    if (ok !== undefined) toasts.push("info", `Baixando ${items.length} item(ns) — acompanhe no painel`);
  }

  async function upload(directory: boolean) {
    if (!remote) return;
    const picked = await open({ directory, multiple: true, title: directory ? "Enviar pastas" : "Enviar arquivos" });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    if (paths.length === 0) return;
    const ok = await app.act(() => api.uploadItems(remote!, path, paths));
    if (ok !== undefined) toasts.push("info", `Enviando ${paths.length} item(ns) — acompanhe no painel`);
  }

  async function createFolder() {
    const name = newFolder?.trim();
    if (!remote || !name) return;
    const target = path ? `${path}/${name}` : name;
    const ok = await app.act(() => api.makeRemoteDir(remote!, target));
    newFolder = null;
    if (ok !== undefined) await load(remote, path);
  }

  async function changeBase() {
    if (!remote) return;
    const dir = await open({ directory: true, title: "Pasta local desta conta" });
    if (typeof dir !== "string") return;
    const c = await app.act(() => api.setFilesBase(remote!, dir), "Pasta local alterada");
    if (c) config = c;
  }

  /** Na modificação, a nuvem ainda é verificada periodicamente (o provedor não avisa o app). */
  const CLOUD_CHECK_ON_CHANGE = 15;

  async function changeInterval(value: string) {
    if (!remote) return;
    const onChange = value === "change";
    const minutes = onChange ? CLOUD_CHECK_ON_CHANGE : Number(value);
    const c = await app.act(() => api.setFilesInterval(remote!, minutes, onChange));
    if (c) config = c;
  }

  function localPathOf(p: string) {
    if (!config) return "";
    const sep = config.base.includes("\\") ? "\\" : "/";
    return p ? `${config.base}${sep}${p.split("/").join(sep)}` : config.base;
  }

  function when(e: FileEntry) {
    if (e.modTime) return formatDateTime(Date.parse(e.modTime));
    return formatDateTime(e.localModMs);
  }
</script>

<div class="flex h-full flex-col">
<ViewHeader
  num="02"
  crumb="arquivos"
  title={remote ? `${remote}:/${path}` : "nenhuma conta"}
  subtitle={remote ? "escolha o que sincroniza, fica offline ou é ignorado" : "conecte uma conta primeiro"}
>
  {#snippet legend()}
    {#each STATES as s (s)}
      <span title={ITEM_STATE[s].help}><span class={ITEM_STATE[s].cls}>{ITEM_STATE[s].icon}</span> {ITEM_STATE[s].label}</span>
    {/each}
  {/snippet}
</ViewHeader>

<div class="grid min-h-0 flex-1 grid-cols-[230px_minmax(0,1fr)] border-t border-line-soft">
  <!-- contas e configuração da cópia local -->
  <aside class="flex min-h-0 flex-col gap-3 overflow-auto border-r border-line-soft px-4 py-4">
    <div class="sec-label">contas</div>
    {#each app.remotes as r (r.name)}
      <button
        class="flex items-center gap-2 border px-3 py-2 text-left text-[12.5px] {remote === r.name
          ? 'border-accent-dim bg-node text-fg'
          : 'border-transparent text-muted hover:bg-node'}"
        onclick={() => ((remote = r.name), (path = ""))}
      >
        <Cloud size={14} class={remote === r.name ? "text-accent" : ""} />
        <span class="truncate">{r.name}</span>
        <span class="ml-auto text-[10.5px] text-dim">{providerLabel(r.kind)}</span>
      </button>
    {/each}
    {#if app.remotes.length === 0}
      <button class="btn" onclick={() => (app.view = "accounts")}>conectar conta</button>
    {/if}

    {#if config}
      {#if ondemand?.supported}
        <div class="sec-label accent mt-4">sob demanda</div>
        {#if ondemand.enabled}
          <div class="flex items-center gap-2 text-[11.5px]">
            <span class={ondemand.connected ? "text-ok" : "text-warn"}>{ondemand.connected ? "◆" : "◇"}</span>
            <span class="text-muted">{ondemand.connected ? "no Explorer como" : "desconectado:"}</span>
          </div>
          <div class="text-[12px] font-semibold">HSync - {remote}</div>
          <div class="selectable text-[11px] break-all text-dim" title={ondemand.path ?? ""}>{ondemand.path}</div>
          <div class="flex gap-2">
            <button class="btn h-7 flex-1 justify-center text-[11px]" onclick={() => ondemand?.path && revealItemInDir(ondemand.path)}>
              <FolderOpen size={12} /> abrir
            </button>
            <button class="btn btn-danger h-7 flex-1 justify-center text-[11px]" onclick={disableOndemand} disabled={ondemandBusy}>desativar</button>
          </div>
          <p class="text-[11px] leading-snug text-dim">
            tudo da nuvem aparece lá sem ocupar espaço e baixa ao abrir. Botão direito: "Sempre manter neste dispositivo" ou
            "Liberar espaço". Por enquanto a pasta é somente leitura.
          </p>
        {:else}
          <p class="text-[11px] leading-snug text-muted">
            veja todos os arquivos da conta numa pasta do Explorer, como no OneDrive — eles só são baixados quando você
            abre.
          </p>
          <button class="btn h-7 justify-center text-[11px]" onclick={enableOndemand} disabled={ondemandBusy}>
            <Cloud size={12} /> {ondemandBusy ? "ativando…" : "ativar arquivos sob demanda"}
          </button>
        {/if}
      {/if}

      <div class="sec-label accent mt-4">cópia local</div>
      <div class="selectable text-[11.5px] break-all text-muted" title={config.base}>{config.base}</div>
      <div class="flex gap-2">
        <button class="btn h-7 flex-1 justify-center text-[11px]" onclick={changeBase}>alterar</button>
        <button class="btn h-7 flex-1 justify-center text-[11px]" onclick={() => revealItemInDir(config!.base).catch(() => toasts.push("info", "A pasta será criada na primeira sincronização"))}>
          <FolderOpen size={12} /> abrir
        </button>
      </div>

      <div class="sec-label mt-3">sincronizar</div>
      <select
        class="input h-7 text-[11.5px]"
        value={config.onChange ? "change" : String(config.intervalMinutes)}
        onchange={(e) => changeInterval(e.currentTarget.value)}
      >
        <option value="change">na modificação</option>
        {#each INTERVALS as m (m)}
          <option value={String(m)}>{m === 0 ? "só manual" : `a cada ${formatInterval(m)}`}</option>
        {/each}
      </select>
      {#if config.onChange}
        <p class="text-[11px] leading-snug text-dim">
          alterações neste PC sincronizam em segundos; mudanças feitas na nuvem são verificadas a cada
          {CLOUD_CHECK_ON_CHANGE} min.
        </p>
      {/if}

      <div class="sec-label mt-3">marcados</div>
      <div class="flex flex-col gap-1 text-[11.5px]">
        {#each STATES as s (s)}
          <div class="flex justify-between">
            <span><span class={ITEM_STATE[s].cls}>{ITEM_STATE[s].icon}</span> {ITEM_STATE[s].label}</span>
            <span class="text-muted">{counts[s]}</span>
          </div>
        {/each}
      </div>

      {#if config.jobs.length}
        <div class="sec-label mt-3">tarefas desta conta</div>
        {#each config.jobs as j (j.id)}
          <div class="text-[11.5px]">
            <div class="flex items-center gap-2">
              <span class={j.running ? "text-ok" : "text-dim"}>{j.running ? "◆" : "◇"}</span>
              <span class="truncate">{j.name}</span>
            </div>
            {#if j.lastRun}
              <div class="pl-5 {STATUS_COLOR[j.lastRun.status]}">{STATUS_LABEL[j.lastRun.status]}</div>
            {/if}
          </div>
        {/each}
        <button class="btn mt-1 justify-center" onclick={() => app.act(() => api.syncAccountNow(remote!))}>
          <RefreshCw size={13} /> sincronizar agora
        </button>
      {/if}
    {/if}
  </aside>

  <!-- navegador -->
  <section class="flex min-h-0 min-w-0 flex-col gap-3 px-5 py-4">
    {#if remote}
      <div class="flex flex-wrap items-center gap-2">
        <div class="flex min-w-0 flex-1 flex-wrap items-center gap-1 text-[12.5px]">
          <button class="text-accent hover:underline" onclick={() => (path = "")}>{remote}:</button>
          {#each crumbs as c, i (i)}
            <span class="text-dim">/</span>
            <button class="hover:underline" onclick={() => (path = crumbs.slice(0, i + 1).join("/"))}>{c}</button>
          {/each}
        </div>
        <button class="btn btn-icon" title="Atualizar" onclick={() => load(remote!, path)}><RefreshCw size={14} /></button>
        <button class="btn" onclick={() => (newFolder = "")}><FolderPlus size={14} /> pasta</button>
        <button class="btn" onclick={() => upload(false)}><Upload size={14} /> enviar arquivos</button>
        <button class="btn btn-icon" title="Enviar pastas" onclick={() => upload(true)}><FolderOpen size={14} /></button>
        <button
          class="btn"
          onclick={freeSelected}
          disabled={freeing || freeable.length === 0}
          title="Remove a cópia local (vai para a Lixeira) dos selecionados que estão nos dois lados e não estão marcados para ficar offline"
        >
          <HardDrive size={14} /> liberar espaço{freeable.length ? ` (${freeable.length})` : ""}
        </button>
        <button class="btn btn-primary" onclick={downloadSelected} disabled={selectedEntries.filter((e) => e.inCloud).length === 0}>
          <Download size={14} /> baixar ({selectedEntries.length})
        </button>
      </div>

      {#if newFolder !== null}
        <div class="flex gap-2">
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input" bind:value={newFolder} placeholder="nome da nova pasta" autofocus onkeydown={(e) => (e.key === "Enter" ? createFolder() : e.key === "Escape" && (newFolder = null))} />
          <button class="btn" onclick={createFolder}>criar</button>
          <button class="btn btn-ghost" onclick={() => (newFolder = null)}>cancelar</button>
        </div>
      {/if}

      <div class="term flex min-h-0 flex-1 flex-col">
        <div class="term-bar shrink-0">
          <span class="dots"><i></i><i></i><i></i></span>
          <span class="mx-auto">rclone — operations/list — {entries.length} itens</span>
        </div>
        <div class="grid shrink-0 grid-cols-[28px_minmax(0,1fr)_90px_110px_96px_170px] items-center gap-x-3 border-b border-line px-3 py-1.5 text-[11px] text-dim">
          <span></span><span>nome</span><span class="text-right">tamanho</span><span>modificado</span><span>onde</span><span>estado</span>
        </div>
        <div class="min-h-0 flex-1 overflow-auto">
          {#if path}
            <button class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-muted hover:bg-node" onclick={() => (path = crumbs.slice(0, -1).join("/"))}>
              <ArrowUp size={14} class="ml-[38px]" /> ..
            </button>
          {/if}
          {#if loading}
            <div class="flex items-center gap-2 px-4 py-3 text-muted"><LoaderCircle size={14} class="spin" /> listando {remote}:/{path}…</div>
          {:else if error}
            <div class="selectable px-4 py-3 text-err">✕ {error}</div>
          {:else if entries.length === 0}
            <div class="px-4 py-3 text-dim">◇ pasta vazia</div>
          {:else}
            {#each entries as e (e.path)}
              {@const eff = effective(e.path)}
              <div class="grid grid-cols-[28px_minmax(0,1fr)_90px_110px_96px_170px] items-center gap-x-3 px-3 py-1 text-[12.5px] hover:bg-node/60 {selected.has(e.path) ? 'bg-accent/5' : ''}">
                <input type="checkbox" class="accent-[#5cb8e4]" checked={selected.has(e.path)} onchange={() => toggle(e.path)} aria-label="Selecionar {e.name}" />
                <span class="flex min-w-0 items-center gap-2">
                  {#if e.isDir}
                    <Folder size={14} class="shrink-0 text-accent" />
                    <button class="truncate text-left hover:underline" onclick={() => (path = e.path)} title={e.name}>{e.name}</button>
                  {:else}
                    <File size={14} class="shrink-0 text-muted" />
                    <button class="truncate text-left hover:underline" ondblclick={() => openPreview(e)} title="{e.name} — duplo clique para pré-visualizar">{e.name}</button>
                    <button class="btn btn-ghost btn-icon ml-auto h-6 w-6 shrink-0" onclick={() => openPreview(e)} title="Pré-visualizar" aria-label="Pré-visualizar {e.name}">
                      <Eye size={13} />
                    </button>
                  {/if}
                </span>
                <span class="text-right text-[11.5px] text-muted">{e.isDir ? "" : formatBytes(e.size)}</span>
                <span class="text-[11.5px] text-muted">{when(e)}</span>
                <span class="flex items-center gap-1.5 text-[11px]">
                  {#if accountSyncing && !e.local && e.inCloud && keepsLocal(eff?.state)}
                    <LoaderCircle size={12} class="spin text-accent" /> <span class="text-accent">baixando…</span>
                  {:else if e.inCloud && e.local}
                    <span class="text-ok" title="Na nuvem e em {localPathOf(e.path)}">◆</span> nos dois
                  {:else if e.inCloud}
                    <Cloud size={12} class="text-dim" /> <span class="text-dim">só nuvem</span>
                  {:else}
                    <HardDrive size={12} class="text-warn" /> <span class="text-warn">só local</span>
                  {/if}
                </span>
                <span class="flex items-center gap-1.5">
                  {#if busyPath === e.path}
                    <LoaderCircle size={13} class="spin text-accent" />
                  {/if}
                  <select
                    class="input h-6 py-0 text-[11.5px] {eff ? ITEM_STATE[eff.state].cls : 'text-dim'} {eff && !eff.explicit ? 'opacity-70' : ''}"
                    value={eff?.explicit ? eff.state : ""}
                    disabled={busyPath !== null}
                    onchange={(ev) => setState(e, ev.currentTarget.value)}
                    title={eff && !eff.explicit ? `herdado de ${eff.from}` : eff ? ITEM_STATE[eff.state].help : "não sincronizado"}
                  >
                    <option value="">{eff && !eff.explicit ? `${ITEM_STATE[eff.state].icon} herdado` : "— nenhum"}</option>
                    {#each STATES as s (s)}
                      <option value={s}>{ITEM_STATE[s].icon} {ITEM_STATE[s].label}</option>
                    {/each}
                  </select>
                </span>
              </div>
            {/each}
          {/if}
        </div>
      </div>
      <p class="text-[11px] text-dim">
        ◇ ao marcar um item, o HSync cria/ajusta a tarefa da conta e já começa a sincronizar. Itens dentro de uma pasta
        marcada herdam o estado dela — marque um subitem para sobrescrever (ex.: ⊘ ignorar uma subpasta).
      </p>
    {/if}
  </section>
</div>
</div>

{#if previewIndex !== null && remote && fileEntries.length}
  <PreviewModal {remote} files={fileEntries} bind:index={previewIndex} onclose={() => (previewIndex = null)} />
{/if}
