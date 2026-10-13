<script lang="ts">
  import { onMount } from "svelte";
  import Gate from "./lib/components/Gate.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import HMark from "./lib/components/ui/HMark.svelte";
  import Toasts from "./lib/components/ui/Toasts.svelte";
  import Accounts from "./lib/components/views/Accounts.svelte";
  import Dashboard from "./lib/components/views/Dashboard.svelte";
  import Files from "./lib/components/views/Files.svelte";
  import Jobs from "./lib/components/views/Jobs.svelte";
  import Settings from "./lib/components/views/Settings.svelte";
  import { app } from "./lib/stores/app.svelte";
  import { VIEWS } from "./lib/views";


  // Durante um reinício o app continua navegável (dados podem estar defasados).
  const gated = $derived(!["ready", "restarting"].includes(app.daemon.state));

  onMount(() => {
    void app.init();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.ctrlKey && !e.shiftKey && !e.altKey && /^[1-5]$/.test(e.key)) {
      e.preventDefault();
      app.view = VIEWS[Number(e.key) - 1].id;
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="flex h-full flex-col gap-3 px-4 pt-3 pb-4">
  <header class="flex h-8 shrink-0 items-center gap-3 px-1">
    <HMark size={24} class="text-accent" />
    <span class="text-[16px] font-bold tracking-tight">hsync</span>

    <nav class="ml-auto flex items-center gap-1 text-[12.5px]" aria-label="Menu principal">
      {#each VIEWS as v, i (v.id)}
        <button
          class="relative px-3 py-1 transition-colors {app.view === v.id ? 'text-fg' : 'text-muted hover:text-fg'}"
          onclick={() => (app.view = v.id)}
          disabled={gated}
          title="{v.summary} (Ctrl+{i + 1})"
          aria-current={app.view === v.id ? "page" : undefined}
        >
          <span class={app.view === v.id ? "text-accent" : "text-dim"}>{i}:</span>{v.label}
          {#if app.view === v.id}<span class="absolute right-3 -bottom-0.5 left-3 h-px bg-accent"></span>{/if}
        </button>
      {/each}
    </nav>
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
      {:else if app.view === "files"}
        <Files />
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
