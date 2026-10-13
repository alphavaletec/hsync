<script lang="ts">
  import { onMount } from "svelte";
  import Gate from "./lib/components/Gate.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import HMark from "./lib/components/ui/HMark.svelte";
  import Toasts from "./lib/components/ui/Toasts.svelte";
  import Accounts from "./lib/components/views/Accounts.svelte";
  import Dashboard from "./lib/components/views/Dashboard.svelte";
  import Jobs from "./lib/components/views/Jobs.svelte";
  import Settings from "./lib/components/views/Settings.svelte";
  import { app, type View } from "./lib/stores/app.svelte";

  const CRUMB: Record<View, string> = { dashboard: "painel", jobs: "tarefas", accounts: "contas", settings: "config" };
  const ORDER: View[] = ["dashboard", "jobs", "accounts", "settings"];

  // Durante um reinício o app continua navegável (dados podem estar defasados).
  const gated = $derived(!["ready", "restarting"].includes(app.daemon.state));

  onMount(() => {
    void app.init();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.ctrlKey && !e.shiftKey && !e.altKey && /^[1-4]$/.test(e.key)) {
      e.preventDefault();
      app.view = ORDER[Number(e.key) - 1];
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="flex h-full flex-col gap-3 px-4 pt-3 pb-4">
  <header class="flex h-8 shrink-0 items-center gap-3 px-1">
    <HMark size={24} class="text-accent" />
    <span class="text-[16px] font-bold tracking-tight">hsync</span>
    <span class="text-dim">/</span>
    <span class="text-muted">{gated ? "rclone" : CRUMB[app.view]}</span>
  </header>

  <main class="flex min-h-0 flex-1 flex-col border border-line bg-frame/95 shadow-[0_20px_60px_rgb(0_0_0/0.35)]">
    <div class="flex h-9 shrink-0 items-center border-b border-line px-4 text-[12px]">
      <span class="text-accent">~/hsync</span>
      <span class="mx-auto text-muted">
        {#if app.daemon.state === "ready"}rclone {app.daemon.version} · rcd 127.0.0.1{:else}rclone rcd{/if}
      </span>
      <span class="text-dim">{app.daemon.state === "ready" ? `:${app.daemon.port}` : ""}</span>
    </div>

    {#if app.daemon.state === "restarting"}
      <div class="shrink-0 border-b border-warn/30 bg-warn/10 px-5 py-1.5 text-[12px] text-warn">
        ◇ reiniciando o rclone (tentativa {app.daemon.attempt}): {app.daemon.reason}
      </div>
    {/if}

    <div class="min-h-0 flex-1 overflow-auto">
      {#if gated}
        <Gate />
      {:else if app.view === "dashboard"}
        <Dashboard />
      {:else if app.view === "jobs"}
        <Jobs />
      {:else if app.view === "accounts"}
        <Accounts />
      {:else}
        <Settings />
      {/if}
    </div>

    <StatusBar />
  </main>
</div>

<Toasts />
