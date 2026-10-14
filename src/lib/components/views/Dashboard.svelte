<script lang="ts">
  import { Pause, Play, PlayCircle } from "@lucide/svelte";
  import { api } from "../../api/commands";
  import { app } from "../../stores/app.svelte";
  import { formatBytes, formatRelative, formatSpeed } from "../../utils/format";
  import LogPanel from "../dashboard/LogPanel.svelte";
  import TopologyMap from "../dashboard/TopologyMap.svelte";
  import TransferDashboard from "../dashboard/TransferDashboard.svelte";
  import Collapsible from "../ui/Collapsible.svelte";
  import ViewHeader from "../ui/ViewHeader.svelte";

  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  const active = $derived(app.progress);
  const overallPct = $derived.by(() => {
    const total = active.reduce((a, p) => a + p.totalBytes, 0);
    const done = active.reduce((a, p) => a + p.bytes, 0);
    return total > 0 ? Math.round((done / total) * 100) : 0;
  });
  const nextJob = $derived(
    app.jobs.filter((j) => j.nextRunAt && !j.running).sort((a, b) => a.nextRunAt! - b.nextRunAt!)[0],
  );
  const today = $derived.by(() => {
    const start = new Date(now);
    start.setHours(0, 0, 0, 0);
    const runs = app.runs.filter((r) => r.startedAt >= start.getTime() && !r.dryRun);
    return {
      runs: runs.length,
      ok: runs.filter((r) => r.status === "success").length,
      bytes: runs.reduce((a, r) => a + r.bytes, 0),
      files: runs.reduce((a, r) => a + r.files, 0),
      errors: runs.filter((r) => r.status === "failed").length,
    };
  });

  const title = $derived(
    active.length > 0
      ? `${active.length} ${active.length === 1 ? "tarefa" : "tarefas"} em execução`
      : app.jobs.length > 0
        ? "tudo em dia"
        : "bem-vindo ao hsync",
  );
  const subtitle = $derived(
    active.length > 0
      ? `${formatSpeed(app.totalSpeed)} · ${overallPct}% concluído`
      : nextJob
        ? `próxima: ${nextJob.name} ${formatRelative(nextJob.nextRunAt, now)}`
        : app.remotes.length === 0
          ? "conecte uma conta para começar"
          : app.jobs.length === 0
            ? "crie sua primeira tarefa"
            : "nenhum agendamento ativo",
  );
</script>

<ViewHeader num="01" crumb="painel" {title} {subtitle}>
  {#snippet legend()}
    <span><span class="text-ok">◆</span> ativo / ok</span>
    <span><span class="text-dim">◇</span> concluído / outros</span>
  {/snippet}
  {#snippet actions()}
    <button class="btn" onclick={() => app.act(() => api.runAllJobs())} disabled={app.jobs.length === 0}>
      <PlayCircle size={14} /> executar todas
    </button>
    {#if app.settings}
      <button
        class="btn"
        onclick={() => app.act(() => api.setSchedulesPaused(!app.settings!.schedulesPaused))}
        title="Pausa/retoma as execuções automáticas"
      >
        {#if app.settings.schedulesPaused}<Play size={14} /> retomar agenda{:else}<Pause size={14} /> pausar agenda{/if}
      </button>
    {/if}
  {/snippet}
</ViewHeader>

<section class="border-t border-line-soft px-5 pt-4 pb-5">
  <div class="mb-4 flex items-center justify-between">
    <div class="sec-label accent">mapa de sincronização</div>
    <div class="flex gap-4 text-[11px] text-dim">
      <span><span class="text-accent">━</span> sincronizando</span>
      <span><span class="text-[#3b4248]">━</span> ociosa</span>
      <span><span class="text-err">┅</span> falhou</span>
      <span>este PC ← rclone → nuvem</span>
    </div>
  </div>
  <TopologyMap />
</section>

<Collapsible id="transferencias" title="transferências">
  {#snippet summary()}
    {active.length > 0 ? `${active.length} em execução · ` : ""}hoje: {today.runs}
    {today.runs === 1 ? "execução" : "execuções"}, {formatBytes(today.bytes)}, {today.files} arq.{today.errors
      ? ` · ${today.errors} ${today.errors === 1 ? "falha" : "falhas"}`
      : ""}
  {/snippet}
  <div class="grid min-w-0 grid-cols-[minmax(0,1fr)_240px] gap-6">
    <TransferDashboard />
    <aside class="flex flex-col gap-3 border-l border-line-soft pl-5">
      <div class="sec-label">hoje</div>
      <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-[12px]">
        <dt class="text-muted">execuções</dt>
        <dd class="text-right">{today.runs} <span class="text-ok">({today.ok} ok)</span></dd>
        <dt class="text-muted">transferido</dt>
        <dd class="text-right">{formatBytes(today.bytes)}</dd>
        <dt class="text-muted">arquivos</dt>
        <dd class="text-right">{today.files}</dd>
        <dt class="text-muted">falhas</dt>
        <dd class="text-right {today.errors ? 'text-err' : ''}">{today.errors}</dd>
      </dl>
    </aside>
  </div>
</Collapsible>

<Collapsible id="log" title="log">
  {#snippet summary()}
    {@const last = app.logs.at(-1)}
    {last ? last.msg : "sem mensagens"}
  {/snippet}
  <LogPanel />
</Collapsible>
