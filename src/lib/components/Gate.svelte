<script lang="ts">
  // Telas exibidas enquanto o daemon não está pronto: inicialização, desbloqueio, falha.
  import { LoaderCircle, Lock, RotateCw } from "@lucide/svelte";
  import { api, errorMessage } from "../api/commands";
  import { app } from "../stores/app.svelte";
  import ViewHeader from "./ui/ViewHeader.svelte";

  let password = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function unlock() {
    busy = true;
    error = null;
    try {
      await api.unlockConfig(password);
      password = "";
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if app.daemon.state === "locked"}
  <ViewHeader num="00" crumb="bloqueado" title="configuração bloqueada" subtitle="informe a senha mestre" />
  <section class="border-t border-line-soft px-5 py-6">
    <div class="term max-w-[480px]">
      <div class="term-bar"><span class="dots"><i></i><i></i><i></i></span><span class="mx-auto">rclone — config/unlock</span></div>
      <form class="flex flex-col gap-3 p-5" onsubmit={(e) => (e.preventDefault(), unlock())}>
        <p class="text-[12px] text-muted"><Lock size={13} class="mr-1 inline text-warn" /> o rclone.conf está criptografado.</p>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="input" type="password" bind:value={password} placeholder="senha mestre" autocomplete="current-password" autofocus />
        {#if error}<p class="text-[12px] text-err">✕ {error}</p>{/if}
        <button class="btn btn-primary self-start" disabled={busy || !password}>{busy ? "verificando…" : "desbloquear"}</button>
      </form>
    </div>
  </section>
{:else if app.daemon.state === "failed"}
  <ViewHeader num="00" crumb="erro" title="o rclone não iniciou" subtitle="várias tentativas falharam" />
  <section class="border-t border-line-soft px-5 py-6">
    <p class="selectable max-w-[720px] text-[12.5px] text-err">✕ {app.daemon.reason}</p>
    <button class="btn btn-primary mt-4" onclick={() => api.daemonRetry()}><RotateCw size={14} /> tentar novamente</button>
  </section>
{:else}
  <div class="flex h-full flex-col items-center justify-center gap-3 text-muted">
    <LoaderCircle size={22} class="spin text-accent" />
    <p class="text-[13px]">
      {#if app.daemon.state === "maintenance"}aplicando alterações na configuração…
      {:else if app.daemon.state === "stopped"}encerrando…
      {:else}iniciando o rclone<span class="blink">_</span>{/if}
    </p>
  </div>
{/if}
