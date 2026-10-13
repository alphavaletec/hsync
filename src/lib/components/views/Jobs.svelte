<script lang="ts">
  import { Cloud, FlaskConical, HardDrive, Pencil, Play, Plus, RefreshCw, Square, Trash2 } from "@lucide/svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api } from "../../api/commands";
  import type { Job } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import {
    MODE_LABEL,
    STATUS_COLOR,
    STATUS_LABEL,
    directionOf,
    formatBytes,
    formatInterval,
    formatRelative,
    isLocalPath,
  } from "../../utils/format";
  import JobForm from "../jobs/JobForm.svelte";
  import ViewHeader from "../ui/ViewHeader.svelte";

  let editing = $state<Job | null | undefined>(undefined); // undefined = fechado, null = nova
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  const scheduled = $derived(app.jobs.filter((j) => j.schedule.enabled).length);

  async function remove(job: Job) {
    const ok = await ask(`Excluir a tarefa "${job.name}"? Os arquivos não são afetados.`, {
      title: "Excluir tarefa",
      kind: "warning",
      okLabel: "Excluir",
      cancelLabel: "Cancelar",
    });
    if (ok) await app.act(() => api.deleteJob(job.id), "Tarefa excluída");
  }

  async function run(job: Job, dryRun = false) {
    if (!dryRun && job.mode === "sync" && !job.lastRun) {
      const ok = await ask(
        `"${job.name}" é um espelho: arquivos que não existem na origem serão APAGADOS do destino. Executar agora? (Recomendado: simular antes.)`,
        { title: "Primeira execução", kind: "warning", okLabel: "Executar", cancelLabel: "Cancelar" },
      );
      if (!ok) return;
    }
    await app.act(() => api.runJob(job.id, { dryRun }));
  }

  async function resync(job: Job) {
    const ok = await ask(
      `Ressincronizar "${job.name}"? Os dois lados são unidos (prevalece a origem em conflitos) e o estado do bidirecional é recriado.`,
      { title: "Ressincronizar", kind: "info", okLabel: "Ressincronizar", cancelLabel: "Cancelar" },
    );
    if (ok) await app.act(() => api.runJob(job.id, { resync: true }));
  }
</script>

<ViewHeader
  num="02"
  crumb="tarefas"
  title={app.jobs.length ? `${app.jobs.length} ${app.jobs.length === 1 ? "tarefa" : "tarefas"}` : "nenhuma tarefa ainda"}
  subtitle={app.jobs.length ? `${scheduled} com agendamento` : "escolha origem, destino e modo"}
>
  {#snippet legend()}
    <span><span class="text-accent">━</span> origem</span>
    <span><span class="text-ok">━</span> destino</span>
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-primary" onclick={() => (editing = null)}><Plus size={14} /> nova tarefa</button>
  {/snippet}
</ViewHeader>

<section class="flex flex-col gap-3 border-t border-line-soft px-5 py-4">
  {#if app.jobs.length === 0}
    <div class="term px-5 py-6 text-[12.5px] leading-[1.9]">
      <p><span class="text-ok">◆</span> 1. conecte uma conta em <span class="text-accent">2:contas</span></p>
      <p><span class="text-ok">◆</span> 2. crie uma tarefa: <span class="text-muted">origem → destino + modo</span></p>
      <p><span class="text-ok">◆</span> 3. simule (dry-run) e execute</p>
      <p class="text-dim">◇ opcional: agende para rodar sozinho na bandeja</p>
    </div>
  {/if}

  {#each app.jobs as job (job.id)}
    <article class="node px-4 py-3" class:border-accent-dim={job.running}>
      <span class="node-dot" class:bg-ok={job.running}></span>
      <div class="flex items-center gap-3">
        <span class={job.running ? "text-ok" : "text-dim"}>{job.running ? "◆" : "◇"}</span>
        <h3 class="truncate text-[14px] font-semibold">{job.name}</h3>
        <span class="badge text-accent">{MODE_LABEL[job.mode]}</span>
        <span class="text-[11px] text-dim">{directionOf(job.source, job.dest)}</span>
        {#if job.mode === "bisync" && !job.bisyncInitialized}
          <span class="badge text-warn" title="A próxima execução fará a ressincronização inicial">resync pendente</span>
        {/if}

        <div class="ml-auto flex items-center gap-1">
          {#if job.running}
            <button class="btn btn-danger" onclick={() => app.act(() => api.stopJob(job.id))}><Square size={13} /> parar</button>
          {:else}
            <button class="btn" onclick={() => run(job)} disabled={!app.ready}><Play size={13} /> executar</button>
            <button class="btn btn-icon" onclick={() => run(job, true)} disabled={!app.ready} title="Simular (dry-run)">
              <FlaskConical size={14} />
            </button>
            {#if job.mode === "bisync" && job.bisyncInitialized}
              <button class="btn btn-icon" onclick={() => resync(job)} disabled={!app.ready} title="Ressincronizar">
                <RefreshCw size={14} />
              </button>
            {/if}
            <button class="btn btn-icon" onclick={() => (editing = job)} title="Editar"><Pencil size={14} /></button>
            <button class="btn btn-icon btn-danger" onclick={() => remove(job)} title="Excluir"><Trash2 size={14} /></button>
          {/if}
        </div>
      </div>

      <div class="mt-2 grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3 text-[12px]">
        <span class="flex min-w-0 items-center gap-2 border-l-2 border-accent pl-2">
          {#if isLocalPath(job.source)}<HardDrive size={13} class="shrink-0 text-muted" />{:else}<Cloud size={13} class="shrink-0 text-accent" />{/if}
          <span class="selectable truncate" title={job.source}>{job.source}</span>
        </span>
        <span class="text-dim">{job.mode === "bisync" ? "⇄" : "→"}</span>
        <span class="flex min-w-0 items-center gap-2 border-l-2 border-ok pl-2">
          {#if isLocalPath(job.dest)}<HardDrive size={13} class="shrink-0 text-muted" />{:else}<Cloud size={13} class="shrink-0 text-accent" />{/if}
          <span class="selectable truncate" title={job.dest}>{job.dest}</span>
        </span>
      </div>

      <div class="mt-2 flex flex-wrap gap-x-6 gap-y-1 text-[11.5px] text-muted">
        <span>
          {#if job.schedule.enabled}
            a cada {formatInterval(job.schedule.intervalMinutes)}
            {#if job.nextRunAt}· próxima {formatRelative(job.nextRunAt, now)}{:else}· <span class="text-warn">pausado</span>{/if}
          {:else}
            execução manual
          {/if}
        </span>
        {#if job.lastRun}
          <span>
            última: <span class={STATUS_COLOR[job.lastRun.status]}>{STATUS_LABEL[job.lastRun.status]}</span>
            {formatRelative(job.lastRun.startedAt, now)}{job.lastRun.dryRun ? " (simulação)" : ""}
            · {formatBytes(job.lastRun.bytes)} · {job.lastRun.files} arq.
          </span>
        {/if}
        {#if job.filters.excludeDirs.length || job.filters.excludeExts.length || job.filters.excludeHidden || job.filters.customRules.length}
          <span class="text-dim">
            filtros: {[
              job.filters.excludeHidden ? "ocultos" : "",
              ...job.filters.excludeDirs.map((d) => d + "/"),
              ...job.filters.excludeExts.map((e) => "." + e.replace(/^\*?\./, "")),
              job.filters.customRules.length ? `${job.filters.customRules.length} regra(s)` : "",
            ]
              .filter(Boolean)
              .join(", ")}
          </span>
        {/if}
      </div>
      {#if job.lastRun?.error && job.lastRun.status !== "success"}
        <p class="selectable mt-1 truncate text-[11.5px] text-err/80" title={job.lastRun.error}>│ {job.lastRun.error}</p>
      {/if}
    </article>
  {/each}
</section>

{#if editing !== undefined}
  <JobForm job={editing} onclose={() => (editing = undefined)} />
{/if}
