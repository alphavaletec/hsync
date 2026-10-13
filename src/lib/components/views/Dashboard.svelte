<script lang="ts">
  import { Pause, Play, PlayCircle } from "@lucide/svelte";
  import { api } from "../../api/commands";
  import { app } from "../../stores/app.svelte";
  import { formatBytes, formatRelative, formatSpeed } from "../../utils/format";
  import FlowDiagram from "../dashboard/FlowDiagram.svelte";
  import LogPanel from "../dashboard/LogPanel.svelte";
  import Sparkline from "../dashboard/Sparkline.svelte";
  import TransferDashboard from "../dashboard/TransferDashboard.svelte";
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
  // Tarefa mostrada no diagrama: a primeira ativa, senão a próxima agendada, senão a última executada.
  const focusJob = $derived(
    app.jobs.find((j) => j.id === active[0]?.jobId) ??
      nextJob ??
      app.jobs.find((j) => j.id === app.runs[0]?.jobId) ??
      app.jobs[0],
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

<div class="grid min-h-0 grid-cols-[minmax(0,1fr)_330px] border-t border-line-soft">
  <section class="flex min-w-0 flex-col gap-3 px-5 py-4">
    <div class="sec-label">transferências</div>
    <TransferDashboard />
    <div class="sec-label mt-2">log</div>
    <LogPanel />
  </section>

  <aside class="flex flex-col gap-3 border-l border-line-soft px-5 py-4">
    <div class="sec-label accent">fluxo</div>
    {#if focusJob}
      <div class="text-center text-[11.5px] text-muted">{focusJob.name}</div>
      <FlowDiagram source={focusJob.source} dest={focusJob.dest} mode={focusJob.mode} active={!!focusJob.running} />
    {:else}
      <p class="text-[12px] text-dim">nenhuma tarefa configurada.</p>
    {/if}

    <div class="sec-label mt-3">velocidade</div>
    <Sparkline samples={app.speedHistory} />

    <div class="sec-label mt-3">hoje</div>
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
