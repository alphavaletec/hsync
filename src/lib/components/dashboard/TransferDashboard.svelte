<script lang="ts">
  // Dashboard de monitoramento: uma "janela de terminal" por tarefa ativa com velocidade,
  // progresso total e arquivos em transferência; histórico recente quando ocioso.
  import { Square } from "@lucide/svelte";
  import { api } from "../../api/commands";
  import { app } from "../../stores/app.svelte";
  import {
    MODE_LABEL,
    STATUS_COLOR,
    STATUS_LABEL,
    formatBytes,
    formatDateTime,
    formatDuration,
    formatSpeed,
  } from "../../utils/format";

  const jobName = (id: number) => app.jobs.find((j) => j.id === id)?.name ?? `#${id}`;
</script>

{#if app.progress.length === 0}
  <div class="term">
    <div class="term-bar">
      <span class="dots"><i></i><i></i><i></i></span>
      <span class="mx-auto">rclone — histórico — {app.runs.length} execuções</span>
    </div>
    <div class="max-h-[340px] overflow-auto px-4 py-3 text-[12.5px] leading-[1.9]">
      {#if app.runs.length === 0}
        <p class="text-muted">◇ nenhuma execução ainda.</p>
        <p class="text-dim">&nbsp;&nbsp;crie uma tarefa em <span class="text-accent">1:tarefas</span> e execute.</p>
      {:else}
        {#each app.runs.slice(0, 30) as r (r.id)}
          <div class="flex items-baseline gap-3 whitespace-nowrap">
            <span class={r.status === "success" ? "text-ok" : "text-dim"}>{r.status === "success" ? "◆" : "◇"}</span>
            <span class="w-[92px] shrink-0 text-dim">{formatDateTime(r.startedAt)}</span>
            <span class="min-w-0 flex-1 truncate">{jobName(r.jobId)}{r.dryRun ? " (simulação)" : ""}</span>
            <span class="w-[86px] shrink-0 text-right text-muted">{formatBytes(r.bytes)}</span>
            <span class="w-[70px] shrink-0 text-right text-muted">{r.files} arq.</span>
            <span class="w-[96px] shrink-0 text-right {STATUS_COLOR[r.status]}">{STATUS_LABEL[r.status]}</span>
          </div>
          {#if r.error && r.status !== "success"}
            <div class="selectable truncate pl-7 text-[11.5px] text-err/80" title={r.error}>│ {r.error}</div>
          {/if}
        {/each}
      {/if}
    </div>
  </div>
{:else}
  <div class="flex flex-col gap-4">
    {#each app.progress as p (p.jobId)}
      <div class="term">
        <div class="term-bar">
          <span class="dots"><i></i><i></i><i></i></span>
          <span class="mx-auto truncate">
            rclone — {MODE_LABEL[p.mode]}{p.dryRun ? " (simulação)" : ""} — {p.jobName}
          </span>
          <button
            class="btn btn-ghost btn-danger h-6 px-2 text-[11px]"
            onclick={() => app.act(() => api.stopJob(p.jobId))}
            title="Parar"
          >
            <Square size={11} /> parar
          </button>
        </div>
        <div class="px-4 py-3">
          <div class="flex items-baseline gap-6">
            <span class="text-[28px] leading-none font-bold">
              {p.percentage.toFixed(0)}<span class="text-[16px] text-muted">%</span>
            </span>
            <span class="text-[15px] font-semibold text-accent">{formatSpeed(p.speed)}</span>
            <span class="text-muted">eta {formatDuration(p.eta)}</span>
            <span class="ml-auto text-muted">decorrido {formatDuration(p.elapsed)}</span>
          </div>
          <div class="progress mt-3" class:indeterminate={p.totalBytes === 0 && p.totalTransfers === 0}>
            <div style:width="{p.percentage}%"></div>
          </div>
          <div class="mt-2 flex flex-wrap gap-x-6 gap-y-1 text-[11.5px] text-muted">
            <span>{formatBytes(p.bytes)} / {formatBytes(p.totalBytes)}</span>
            <span>arquivos {p.transfers}/{p.totalTransfers}</span>
            <span>verificados {p.checks}{p.totalChecks ? `/${p.totalChecks}` : ""}</span>
            {#if p.deletes}<span class="text-warn">excluídos {p.deletes}</span>{/if}
            {#if p.errors}<span class="text-err">erros {p.errors}</span>{/if}
          </div>

          <div class="mt-3 border-l border-line pl-3 text-[12.5px] leading-[1.9]">
            {#if p.transferring.length === 0}
              <div class="text-dim">◇ verificando arquivos<span class="blink">_</span></div>
            {/if}
            {#each p.transferring as t (t.name)}
              <div class="flex items-center gap-3">
                <span class="text-ok">◆</span>
                <span class="min-w-0 flex-1 truncate" title={t.name}>{t.name}</span>
                <span class="progress w-[90px] shrink-0"><div style:width="{t.percentage}%"></div></span>
                <span class="w-[38px] shrink-0 text-right text-muted">{t.percentage}%</span>
                <span class="w-[92px] shrink-0 text-right text-accent">{formatSpeed(t.speed)}</span>
              </div>
            {/each}
          </div>

          {#if p.lastError}
            <div class="selectable mt-2 truncate text-[11.5px] text-err" title={p.lastError}>✕ {p.lastError}</div>
          {/if}
        </div>
      </div>
    {/each}
  </div>
{/if}
