<script lang="ts">
  import { ArrowLeftRight, ArrowRight, Copy, TriangleAlert } from "@lucide/svelte";
  import { api, errorMessage } from "../../api/commands";
  import type { Job, JobInput, SyncMode } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { MODE_HELP, MODE_LABEL, directionOf, formatInterval } from "../../utils/format";
  import ChipsInput from "../ui/ChipsInput.svelte";
  import Modal from "../ui/Modal.svelte";
  import PathPicker from "./PathPicker.svelte";

  let { job = null, onclose }: { job?: Job | null; onclose: () => void } = $props();

  const INTERVALS = [15, 30, 60, 180, 360, 720, 1440];
  const MODES: { id: SyncMode; icon: typeof Copy }[] = [
    { id: "copy", icon: Copy },
    { id: "sync", icon: ArrowRight },
    { id: "bisync", icon: ArrowLeftRight },
  ];

  // Valor inicial intencional: o formulário edita uma cópia.
  // svelte-ignore state_referenced_locally
  const init = job;
  let form = $state<JobInput>({
    name: init?.name ?? "",
    source: init?.source ?? "",
    dest: init?.dest ?? "",
    mode: init?.mode ?? "copy",
    filters: init
      ? $state.snapshot(init.filters)
      : { excludeDirs: [], excludeExts: [], excludeHidden: true, customRules: [] },
    schedule: init ? $state.snapshot(init.schedule) : { enabled: false, intervalMinutes: 60 },
  });
  let customRulesText = $state((init?.filters.customRules ?? []).join("\n"));
  let simulateAfter = $state(!init);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let showAdvanced = $state((init?.filters.customRules.length ?? 0) > 0);

  const direction = $derived(form.source && form.dest ? directionOf(form.source, form.dest) : null);

  async function save() {
    saving = true;
    error = null;
    const input: JobInput = {
      ...$state.snapshot(form),
      filters: {
        ...$state.snapshot(form.filters),
        customRules: customRulesText
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
      },
    };
    try {
      const saved = init ? await api.updateJob(init.id, input) : await api.createJob(input);
      await app.refreshJobs();
      toasts.push("ok", init ? "Tarefa atualizada" : "Tarefa criada");
      if (simulateAfter) {
        await app.act(() => api.runJob(saved.id, { dryRun: true }));
        app.view = "dashboard";
      }
      onclose();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      saving = false;
    }
  }
</script>

<Modal title={init ? `editar tarefa — ${init.name}` : "nova tarefa"} width="760px" {onclose}>
  <div class="flex flex-col gap-5">
    <div>
      <label class="field-label" for="job-name">nome</label>
      <input id="job-name" class="input" bind:value={form.name} placeholder="Backup de fotos" maxlength="80" />
    </div>

    <div class="grid grid-cols-1 gap-4">
      <PathPicker label="origem" bind:value={form.source} />
      <PathPicker label="destino" bind:value={form.dest} />
      {#if direction}
        <div class="text-[11.5px] text-muted">direção detectada: <span class="text-accent">{direction}</span></div>
      {/if}
    </div>

    <div>
      <span class="field-label">modo</span>
      <div class="grid grid-cols-3 gap-3">
        {#each MODES as m (m.id)}
          <button
            class="node px-3 py-3 text-left transition-colors {form.mode === m.id ? 'border-accent' : 'hover:border-accent-dim'}"
            onclick={() => (form.mode = m.id)}
          >
            <span class="node-dot" class:bg-accent={form.mode === m.id}></span>
            <div class="flex items-center gap-2 font-semibold">
              <m.icon size={14} class={form.mode === m.id ? "text-accent" : "text-muted"} />
              {MODE_LABEL[m.id]}
            </div>
            <p class="mt-1 text-[11px] leading-snug text-muted">{MODE_HELP[m.id]}</p>
          </button>
        {/each}
      </div>
      {#if form.mode === "sync"}
        <p class="mt-2 flex items-start gap-2 text-[11.5px] text-warn">
          <TriangleAlert size={14} class="mt-0.5 shrink-0" />
          O espelho apaga do destino tudo que não existir na origem. Faça uma simulação antes da primeira execução.
        </p>
      {:else if form.mode === "bisync"}
        <p class="mt-2 text-[11.5px] text-muted">
          A primeira execução faz uma ressincronização completa (une os dois lados, sem apagar nada). Por segurança o
          rclone aborta se mais de 50% dos arquivos fossem ser excluídos.
        </p>
      {/if}
    </div>

    <div>
      <div class="sec-label mb-3">filtros</div>
      <div class="grid grid-cols-2 gap-4">
        <div>
          <span class="field-label">ignorar pastas (qualquer nível)</span>
          <ChipsInput bind:values={form.filters.excludeDirs} placeholder="node_modules, .cache" />
        </div>
        <div>
          <span class="field-label">ignorar extensões</span>
          <ChipsInput bind:values={form.filters.excludeExts} prefix="." placeholder="tmp, log, part" />
        </div>
      </div>
      <label class="mt-3 flex cursor-pointer items-center gap-2 text-[12px]">
        <input type="checkbox" bind:checked={form.filters.excludeHidden} class="accent-[#5cb8e4]" />
        ignorar ocultos (arquivos que começam com ".", desktop.ini, Thumbs.db, lixeira)
      </label>
      <button class="mt-2 text-[11.5px] text-accent hover:underline" onclick={() => (showAdvanced = !showAdvanced)}>
        {showAdvanced ? "▾" : "▸"} regras avançadas do rclone
      </button>
      {#if showAdvanced}
        <textarea
          class="input mt-2 font-mono"
          rows="3"
          bind:value={customRulesText}
          placeholder={"+ importante/**\n- *.iso"}
        ></textarea>
        <p class="mt-1 text-[11px] text-dim">
          uma regra por linha, aplicadas antes das demais. <span class="text-muted">+</span> inclui,
          <span class="text-muted">-</span> exclui.
        </p>
      {/if}
    </div>

    <div>
      <div class="sec-label mb-3">agendamento</div>
      <div class="flex items-center gap-4">
        <label class="flex cursor-pointer items-center gap-2 text-[12px]">
          <input type="checkbox" bind:checked={form.schedule.enabled} class="accent-[#5cb8e4]" />
          executar automaticamente a cada
        </label>
        <select class="input w-[140px]" bind:value={form.schedule.intervalMinutes} disabled={!form.schedule.enabled}>
          {#each INTERVALS as m (m)}
            <option value={m}>{formatInterval(m)}</option>
          {/each}
          {#if !INTERVALS.includes(form.schedule.intervalMinutes)}
            <option value={form.schedule.intervalMinutes}>{formatInterval(form.schedule.intervalMinutes)}</option>
          {/if}
        </select>
      </div>
      <p class="mt-1 text-[11px] text-dim">o HSync precisa estar aberto (pode ficar na bandeja) para executar no horário.</p>
    </div>

    {#if error}
      <p class="selectable border border-err/40 bg-err/10 px-3 py-2 text-[12px] text-err">✕ {error}</p>
    {/if}
  </div>

  {#snippet footer()}
    <label class="mr-auto flex cursor-pointer items-center gap-2 text-[12px] text-muted">
      <input type="checkbox" bind:checked={simulateAfter} class="accent-[#5cb8e4]" />
      simular depois de salvar (dry-run)
    </label>
    <button class="btn" onclick={onclose}>cancelar</button>
    <button class="btn btn-primary" onclick={save} disabled={saving || !form.name || !form.source || !form.dest}>
      {saving ? "salvando…" : init ? "salvar" : "criar tarefa"}
    </button>
  {/snippet}
</Modal>
