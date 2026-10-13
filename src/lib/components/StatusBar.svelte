<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api } from "../api/commands";
  import { app, type View } from "../stores/app.svelte";
  import { formatSpeed } from "../utils/format";

  const TABS: { id: View; label: string }[] = [
    { id: "dashboard", label: "painel" },
    { id: "jobs", label: "tarefas" },
    { id: "accounts", label: "contas" },
    { id: "settings", label: "config" },
  ];

  const statusText = $derived.by(() => {
    switch (app.daemon.state) {
      case "ready":
        return { text: "rclone pronto", cls: "bg-ok" };
      case "starting":
        return { text: "iniciando rclone", cls: "bg-accent blink" };
      case "restarting":
        return { text: `reiniciando (${app.daemon.attempt})`, cls: "bg-warn blink" };
      case "locked":
        return { text: "bloqueado", cls: "bg-warn" };
      case "maintenance":
        return { text: "manutenção", cls: "bg-warn blink" };
      case "failed":
        return { text: "rclone falhou", cls: "bg-err" };
      default:
        return { text: "parado", cls: "bg-dim" };
    }
  });

  async function quit() {
    if (app.progress.length > 0) {
      const ok = await ask(`${app.progress.length} tarefa(s) em execução serão interrompidas. Sair do HSync?`, {
        title: "Sair",
        kind: "warning",
        okLabel: "Sair",
        cancelLabel: "Cancelar",
      });
      if (!ok) return;
    }
    await api.quitApp();
  }
</script>

<footer class="flex h-8 shrink-0 items-stretch border-t border-line bg-[#16181b] text-[12px]">
  <span class="flex items-center bg-accent px-3 font-semibold text-[#0c1a22]">[hsync]</span>
  <nav class="flex items-stretch">
    {#each TABS as tab, i (tab.id)}
      <button
        class="px-3 transition-colors hover:text-fg {app.view === tab.id ? 'text-fg' : 'text-muted'}"
        onclick={() => (app.view = tab.id)}
        title="Ctrl+{i + 1}"
      >
        {i}:{tab.label}{app.view === tab.id ? "*" : ""}
      </button>
    {/each}
  </nav>

  <div class="ml-auto flex items-center gap-4 px-3 text-muted">
    {#if app.settings?.schedulesPaused}
      <span class="text-warn">agendamentos pausados</span>
    {/if}
    {#if app.progress.length > 0}
      <span class="text-accent">↕ {formatSpeed(app.totalSpeed)}</span>
    {/if}
    <span class="flex items-center gap-2">
      <i class="inline-block h-2 w-2 {statusText.cls}"></i>
      {statusText.text}
    </span>
    <button class="hover:text-err" onclick={quit} title="Encerra as tarefas, o rclone e o app">sair</button>
  </div>
</footer>
