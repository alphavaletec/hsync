<script lang="ts">
  // Mapa de sincronização: pastas locais (esquerda) → motor rclone (centro) → contas (direita).
  // Cada tarefa é uma ligação; tarefas em execução animam o sentido dos dados.
  import { Cloud, Eye, HardDrive } from "@lucide/svelte";
  import type { Job, JobProgress } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import { MODE_LABEL, STATUS_LABEL, formatSpeed, isLocalPath, providerLabel, remoteOf } from "../../utils/format";
  import HMark from "../ui/HMark.svelte";

  type Side = "left" | "right";
  interface MapNode {
    id: string;
    side: Side;
    kind: "local" | "remote";
    label: string;
    sub: string;
    path?: string;
    missing?: boolean;
  }
  interface Placed extends MapNode {
    x: number;
    y: number;
  }
  interface Seg {
    id: string;
    d: string;
    /** Os dados percorrem o caminho no sentido em que ele foi desenhado? */
    forward: boolean;
  }
  interface Edge {
    job: Job;
    progress?: JobProgress;
    segs: Seg[];
    state: "running" | "failed" | "idle" | "paused";
    /** Acabou de concluir com sucesso (brilho verde por alguns segundos). */
    done: boolean;
    /** Ociosa, mas agendada/observando: pulso lento de "conectado". */
    alive: boolean;
  }

  const NODE_H = 58;
  const GAP = 16;
  const HUB_W = 196;
  const HUB_H = 84;
  const PIN = 7;
  const CHAMFER = 8;

  let width = $state(900);
  let hovered = $state<number | null>(null);
  const reduceMotion =
    typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

  const nodeW = $derived(Math.max(170, Math.min(250, width * 0.25)));
  const running = $derived(new Map(app.progress.filter((p) => p.jobId > 0).map((p) => [p.jobId, p])));

  function endpointId(path: string) {
    const r = remoteOf(path);
    return r ? `r:${r}` : `l:${path.toLowerCase()}`;
  }

  function folderName(p: string) {
    const parts = p.split(/[\\/]/).filter(Boolean);
    return parts.at(-1) ?? p;
  }

  // Nós: contas à direita (todas, mesmo sem tarefas) e pastas locais usadas por tarefas à esquerda.
  const nodes = $derived.by(() => {
    const map = new Map<string, MapNode>();
    for (const r of app.remotes) {
      map.set(`r:${r.name}`, { id: `r:${r.name}`, side: "right", kind: "remote", label: r.name, sub: providerLabel(r.kind) });
    }
    for (const j of app.jobs) {
      for (const p of [j.source, j.dest]) {
        const id = endpointId(p);
        if (map.has(id)) continue;
        const r = remoteOf(p);
        map.set(
          id,
          r
            ? { id, side: "right", kind: "remote", label: r, sub: "conta removida", missing: true }
            : { id, side: "left", kind: "local", label: folderName(p), sub: p, path: p },
        );
      }
    }
    return [...map.values()];
  });

  const left = $derived(nodes.filter((n) => n.side === "left"));
  const right = $derived(nodes.filter((n) => n.side === "right"));
  const rows = $derived(Math.max(left.length, right.length, 1));
  const height = $derived(Math.max(rows * (NODE_H + GAP) - GAP, HUB_H) + 24);
  const hub = $derived({ x: width / 2 - HUB_W / 2, y: height / 2 - HUB_H / 2 });

  function place(list: MapNode[], side: Side): Placed[] {
    const total = list.length * (NODE_H + GAP) - GAP;
    const top = (height - total) / 2;
    const x = side === "left" ? PIN + 2 : width - nodeW - PIN - 2;
    return list.map((n, i) => ({ ...n, x, y: top + i * (NODE_H + GAP) }));
  }
  const placed = $derived(new Map([...place(left, "left"), ...place(right, "right")].map((n) => [n.id, n])));

  /** Conector ortogonal com chanfros de 45°, como na referência. */
  function ortho(sx: number, sy: number, ex: number, ey: number): string {
    const mx = (sx + ex) / 2;
    const dy = ey - sy;
    if (Math.abs(dy) < 1) return `M${sx},${sy} H${ex}`;
    const c = Math.min(CHAMFER, Math.abs(dy) / 2, Math.abs(ex - sx) / 4);
    const s = Math.sign(dy);
    return `M${sx},${sy} H${mx - c} L${mx},${sy + s * c} V${ey - s * c} L${mx + c},${ey} H${ex}`;
  }

  function spread(i: number, n: number, span: number) {
    if (n <= 1) return 0;
    const step = Math.min(10, span / (n - 1));
    return (i - (n - 1) / 2) * step;
  }

  const edges = $derived.by((): Edge[] => {
    const paused = app.settings?.schedulesPaused ?? false;
    const list = app.jobs.map((job) => {
      const progress = running.get(job.id);
      const state: Edge["state"] = progress
        ? "running"
        : job.lastRun && ["failed", "interrupted"].includes(job.lastRun.status)
          ? "failed"
          : paused && job.schedule.enabled
            ? "paused"
            : "idle";
      return { job, progress, state, src: placed.get(endpointId(job.source)), dst: placed.get(endpointId(job.dest)) };
    });

    // Índices para espalhar várias ligações no mesmo nó/lado do motor.
    const perNode = new Map<string, number[]>();
    const perHub: Record<Side, { key: number; y: number }[]> = { left: [], right: [] };
    list.forEach((e, i) => {
      for (const n of [e.src, e.dst]) {
        if (!n) continue;
        perNode.set(n.id, [...(perNode.get(n.id) ?? []), i]);
        perHub[n.side].push({ key: i * 2 + (n === e.src ? 0 : 1), y: n.y });
      }
    });
    for (const s of ["left", "right"] as Side[]) perHub[s].sort((a, b) => a.y - b.y || a.key - b.key);

    return list.map((e, i) => {
      const segs: Seg[] = [];
      for (const [n, isSource] of [
        [e.src, true],
        [e.dst, false],
      ] as const) {
        if (!n) continue;
        const atNode = perNode.get(n.id)!;
        const ny = n.y + NODE_H / 2 + spread(atNode.indexOf(i), atNode.length, NODE_H - 24);
        const hubList = perHub[n.side];
        const hi = hubList.findIndex((h) => h.key === i * 2 + (isSource ? 0 : 1));
        const hy = hub.y + HUB_H / 2 + spread(hi, hubList.length, HUB_H - 24);
        const id = `seg-${e.job.id}-${isSource ? "s" : "d"}`;
        if (n.side === "left") {
          // Desenhado nó → motor: os dados seguem esse sentido quando o nó é a origem.
          segs.push({ id, d: ortho(n.x + nodeW + PIN, ny, hub.x - PIN, hy), forward: isSource });
        } else {
          // Desenhado motor → nó: os dados seguem esse sentido quando o nó é o destino.
          segs.push({ id, d: ortho(hub.x + HUB_W + PIN, hy, n.x - PIN, ny), forward: !isSource });
        }
      }
      const finished = app.recentlyFinished[e.job.id];
      return {
        job: e.job,
        progress: e.progress,
        state: e.state,
        segs,
        done: !e.progress && finished?.status === "success",
        // Ligação "viva": agendada ou na modificação, sem pausa — pulso discreto em repouso.
        alive: e.state === "idle" && !paused && (e.job.schedule.enabled || e.job.schedule.onChange),
      };
    });
  });

  // Pastas locais observadas ("na modificação").
  const watchedNodes = $derived(
    new Set(
      app.jobs
        .filter((j) => j.schedule.onChange)
        .flatMap((j) => [j.source, j.dest].filter(isLocalPath).map(endpointId)),
    ),
  );
  const anyAlive = $derived(edges.some((e) => e.alive));

  const activeByNode = $derived.by(() => {
    const m = new Map<string, { speed: number; count: number }>();
    for (const e of edges) {
      if (!e.progress) continue;
      for (const p of [e.job.source, e.job.dest]) {
        const id = endpointId(p);
        const cur = m.get(id) ?? { speed: 0, count: 0 };
        m.set(id, { speed: cur.speed + e.progress.speed, count: cur.count + 1 });
      }
    }
    return m;
  });
  const failedNodes = $derived(
    new Set(edges.filter((e) => e.state === "failed").flatMap((e) => [endpointId(e.job.source), endpointId(e.job.dest)])),
  );
  const jobsPerNode = (id: string) => app.jobs.filter((j) => endpointId(j.source) === id || endpointId(j.dest) === id).length;

  const STROKE: Record<Edge["state"], string> = {
    running: "#5cb8e4",
    failed: "#f06b72",
    idle: "#3b4248",
    paused: "#3b4248",
  };

  function dim(e: Edge) {
    return hovered !== null && hovered !== e.job.id;
  }

  function title(e: Edge) {
    const last = e.job.lastRun ? ` · última: ${STATUS_LABEL[e.job.lastRun.status]}` : "";
    const now = e.progress ? ` · ${e.progress.percentage.toFixed(0)}% · ${formatSpeed(e.progress.speed)}` : "";
    return `${e.job.name} — ${MODE_LABEL[e.job.mode]}${now}${last}`;
  }
</script>

<div class="relative" bind:clientWidth={width} style:height="{height}px">
  <svg class="absolute inset-0 overflow-visible" width={width} height={height} aria-hidden="true">
    {#each edges as e, ei (e.job.id)}
      {#each e.segs as s, si (s.id)}
        <path id={s.id} d={s.d} fill="none" stroke={STROKE[e.state]} stroke-width={hovered === e.job.id ? 2.5 : 1.75}
          stroke-dasharray={e.state === "failed" ? "3 4" : e.state === "paused" ? "2 5" : undefined}
          class:flow-fwd={e.state === "running" && !reduceMotion && s.forward}
          class:flow-rev={e.state === "running" && !reduceMotion && !s.forward}
          class="transition-opacity duration-200" opacity={dim(e) ? 0.25 : 1} />
        {#if e.state === "running" && !reduceMotion}
          <!-- Pacote percorrendo a ligação no sentido dos dados (nos dois sentidos no bidirecional). -->
          <rect width="6" height="6" x="-3" y="-3" fill="#5cb8e4" class="packet">
            <animateMotion dur="1.8s" repeatCount="indefinite" keyPoints={s.forward ? "0;1" : "1;0"} keyTimes="0;1" calcMode="linear">
              <mpath href="#{s.id}" />
            </animateMotion>
          </rect>
          {#if e.job.mode === "bisync"}
            <rect width="5" height="5" x="-2.5" y="-2.5" fill="#3fcf8e" class="packet">
              <animateMotion dur="1.8s" begin="0.9s" repeatCount="indefinite" keyPoints={s.forward ? "1;0" : "0;1"} keyTimes="0;1" calcMode="linear">
                <mpath href="#{s.id}" />
              </animateMotion>
            </rect>
          {/if}
        {/if}
        {#if e.done && !reduceMotion}
          <!-- Conclusão: brilho verde que se apaga (execuções rápidas também ficam visíveis). -->
          <path d={s.d} fill="none" stroke="#3fcf8e" stroke-width="2.5" class="done-flash" />
        {/if}
        {#if e.alive && !reduceMotion}
          <!-- Pulso de repouso: um ponto discreto percorre a ligação no sentido dos dados. -->
          <!-- Ciclo de 9 s: percorre a linha nos primeiros ~2,6 s e fica invisível no resto. -->
          <circle r="2.2" fill="#5cb8e4" opacity="0">
            <animateMotion dur="9s" begin="{(ei * 1.7 + si * 2.6) % 9}s" repeatCount="indefinite"
              keyPoints={s.forward ? "0;1;1" : "1;0;0"} keyTimes="0;0.29;1" calcMode="linear">
              <mpath href="#{s.id}" />
            </animateMotion>
            <animate attributeName="opacity" values="0;0.75;0.75;0;0" keyTimes="0;0.04;0.25;0.29;1" dur="9s"
              begin="{(ei * 1.7 + si * 2.6) % 9}s" repeatCount="indefinite" />
          </circle>
        {/if}
        <!-- Área de hover mais larga que a linha visível. -->
        <path d={s.d} fill="none" stroke="transparent" stroke-width="12" class="cursor-pointer"
          role="presentation" onmouseenter={() => (hovered = e.job.id)} onmouseleave={() => (hovered = null)}>
          <title>{title(e)}</title>
        </path>
      {/each}
    {/each}
  </svg>

  <!-- motor -->
  <div class="node absolute flex flex-col justify-center px-4" class:hub-active={running.size > 0 && !reduceMotion}
    class:hub-idle={running.size === 0 && anyAlive && !reduceMotion}
    style:left="{hub.x}px" style:top="{hub.y}px" style:width="{HUB_W}px" style:height="{HUB_H}px">
    <span class="node-dot" class:bg-accent={running.size > 0}></span>
    <div class="flex items-center gap-3">
      <HMark size={24} class={running.size > 0 ? "text-accent" : "text-fg"} />
      <div class="min-w-0">
        <div class="text-[14px] font-semibold">rclone</div>
        {#if running.size > 0}
          <div class="truncate text-[11px] text-accent">{running.size} sincronizando</div>
          <div class="truncate text-[11px] text-ok"><span class="live">◆</span> {formatSpeed(app.totalSpeed)}</div>
        {:else}
          <div class="truncate text-[11px] text-muted">{app.ready ? "pronto · ocioso" : "indisponível"}</div>
        {/if}
      </div>
    </div>
  </div>

  {#each [...placed.values()] as n (n.id)}
    {@const act = activeByNode.get(n.id)}
    <div class="node absolute flex flex-col justify-center px-3 transition-colors {act ? 'border-accent-dim' : ''} {n.missing ? 'opacity-60' : ''}"
      style:left="{n.x}px" style:top="{n.y}px" style:width="{nodeW}px" style:height="{NODE_H}px"
      title={n.path ?? `${n.label}:`}>
      <span class="node-dot" class:bg-ok={!!act}></span>
      <div class="flex min-w-0 items-center gap-2">
        {#if n.kind === "local"}
          <HardDrive size={15} class="shrink-0 {act ? 'text-accent' : 'text-muted'}" />
        {:else}
          <Cloud size={15} class="shrink-0 {act ? 'text-accent' : n.missing ? 'text-warn' : 'text-muted'}" />
        {/if}
        <span class="truncate text-[12.5px] font-semibold">{n.kind === "remote" ? `${n.label}:` : n.label}</span>
        {#if watchedNodes.has(n.id) && !act}
          <span title="Na modificação: alterações nesta pasta sincronizam em segundos">
            <Eye size={12} class="shrink-0 text-accent {reduceMotion ? '' : 'watching'}" />
          </span>
        {/if}
        {#if failedNodes.has(n.id) && !act}<span class="ml-auto text-[11px] text-err" title="Última execução falhou">✕</span>{/if}
      </div>
      <div class="mt-0.5 truncate text-[11px] text-muted">
        {#if act}
          <span class="text-ok"><span class="live">◆</span> {formatSpeed(act.speed)}</span>
        {:else if n.kind === "remote"}
          {n.sub} · {jobsPerNode(n.id) ? `${jobsPerNode(n.id)} tarefa(s)` : "conectada"}
        {:else}
          {n.sub}
        {/if}
      </div>
    </div>
  {/each}

  {#if left.length === 0}
    <div class="node absolute flex flex-col justify-center px-3 opacity-60"
      style:left="{PIN + 2}px" style:top="{height / 2 - NODE_H / 2}px" style:width="{nodeW}px" style:height="{NODE_H}px">
      <div class="flex items-center gap-2 text-[12.5px] font-semibold"><HardDrive size={15} class="text-muted" /> este PC</div>
      <div class="mt-0.5 text-[11px] text-dim">nenhuma pasta sincronizada</div>
    </div>
  {/if}
  {#if right.length === 0}
    <div class="node absolute flex flex-col justify-center px-3 opacity-60"
      style:left="{width - nodeW - PIN - 2}px" style:top="{height / 2 - NODE_H / 2}px" style:width="{nodeW}px" style:height="{NODE_H}px">
      <div class="flex items-center gap-2 text-[12.5px] font-semibold"><Cloud size={15} class="text-muted" /> nuvem</div>
      <div class="mt-0.5 text-[11px] text-dim">nenhuma conta conectada</div>
    </div>
  {/if}
</div>

{#if edges.length}
  <div class="mt-4 flex flex-wrap gap-2">
    {#each edges as e (e.job.id)}
      <button class="chip cursor-default transition-colors {hovered === e.job.id ? 'border-accent-dim text-fg' : 'text-muted'}"
        onmouseenter={() => (hovered = e.job.id)} onmouseleave={() => (hovered = null)} title={title(e)}>
        <span class={e.state === "running" ? "live text-ok" : e.state === "failed" ? "text-err" : "text-dim"}>
          {e.state === "running" ? "◆" : e.state === "failed" ? "✕" : "◇"}
        </span>
        {e.job.name}
        {#if !e.job.managed}
          <span class="text-dim">{e.job.mode === "bisync" ? "⇄" : isLocalPath(e.job.source) ? "↑" : "↓"}</span>
        {/if}
        {#if e.progress}<span class="text-accent">{e.progress.percentage.toFixed(0)}%</span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .flow-fwd,
  .flow-rev {
    stroke-dasharray: 7 5;
  }
  .flow-fwd {
    animation: dash-fwd 0.9s linear infinite;
  }
  .flow-rev {
    animation: dash-rev 0.9s linear infinite;
  }
  @keyframes dash-fwd {
    to {
      stroke-dashoffset: -12;
    }
  }
  @keyframes dash-rev {
    to {
      stroke-dashoffset: 12;
    }
  }
  .packet {
    filter: drop-shadow(0 0 3px rgb(92 184 228 / 0.8));
  }
  /* Motor "respirando" enquanto há transferências. */
  .hub-active {
    animation: breathe 2.4s ease-in-out infinite;
  }
  @keyframes breathe {
    0%,
    100% {
      box-shadow: 0 0 0 1px rgb(92 184 228 / 0.25), 0 0 12px rgb(92 184 228 / 0.08);
    }
    50% {
      box-shadow: 0 0 0 1px rgb(92 184 228 / 0.55), 0 0 26px rgb(92 184 228 / 0.22);
    }
  }
  /* Em repouso, com ligações ativas: respiração bem mais lenta e fraca. */
  .hub-idle {
    animation: breathe-idle 5s ease-in-out infinite;
  }
  @keyframes breathe-idle {
    0%,
    100% {
      box-shadow: 0 0 0 1px rgb(92 184 228 / 0.06);
    }
    50% {
      box-shadow: 0 0 0 1px rgb(92 184 228 / 0.22), 0 0 14px rgb(92 184 228 / 0.07);
    }
  }
  /* Conclusão: a ligação acende em verde e se apaga. */
  .done-flash {
    animation: done-flash 3.2s ease-out forwards;
    filter: drop-shadow(0 0 4px rgb(63 207 142 / 0.7));
  }
  @keyframes done-flash {
    0% {
      opacity: 0;
    }
    12% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  /* Pasta observada: o olho "pisca" devagar. */
  :global(.watching) {
    animation: watching 3.2s ease-in-out infinite;
  }
  @keyframes watching {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 1;
    }
  }
  .live {
    animation: live 1.6s ease-in-out infinite;
  }
  @keyframes live {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .flow-fwd,
    .flow-rev,
    .hub-active,
    .hub-idle,
    .done-flash,
    .live,
    :global(.watching) {
      animation: none;
    }
  }
</style>
