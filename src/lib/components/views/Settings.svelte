<script lang="ts">
  import { KeyRound, Lock, LockOpen } from "@lucide/svelte";
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { api, errorMessage } from "../../api/commands";
  import { app } from "../../stores/app.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import ViewHeader from "../ui/ViewHeader.svelte";

  let bw = $state(app.settings?.bwlimit ?? "");
  let autostart = $state(false);
  let pwCurrent = $state("");
  let pwNew = $state("");
  let pwConfirm = $state("");
  let pwBusy = $state(false);
  let pwError = $state<string | null>(null);

  const encrypted = $derived(app.settings?.encrypted ?? false);
  const version = $derived(app.daemon.state === "ready" ? app.daemon.version : "—");

  $effect(() => {
    isEnabled().then((v) => (autostart = v)).catch(() => {});
  });

  async function saveBw() {
    const v = await app.act(() => api.setBwlimit(bw), "Limite de banda aplicado");
    if (v !== undefined) {
      bw = v;
      await app.refreshSettings();
    }
  }

  async function toggleAutostart() {
    try {
      if (autostart) await disable();
      else await enable();
      autostart = await isEnabled();
    } catch (e) {
      toasts.push("err", errorMessage(e));
    }
  }

  async function savePassword() {
    pwError = null;
    if (pwNew !== pwConfirm) {
      pwError = "A confirmação não confere";
      return;
    }
    pwBusy = true;
    try {
      await api.setMasterPassword(encrypted ? pwCurrent : null, pwNew);
      toasts.push("ok", encrypted ? "Senha mestre alterada" : "Configuração criptografada com a senha mestre");
      pwCurrent = pwNew = pwConfirm = "";
      await app.refreshSettings();
    } catch (e) {
      pwError = errorMessage(e);
    } finally {
      pwBusy = false;
    }
  }

  async function removePassword() {
    pwError = null;
    pwBusy = true;
    try {
      await api.removeMasterPassword(pwCurrent);
      toasts.push("ok", "Criptografia removida");
      pwCurrent = "";
      await app.refreshSettings();
    } catch (e) {
      pwError = errorMessage(e);
    } finally {
      pwBusy = false;
    }
  }
</script>

<ViewHeader num="04" crumb="config" title="configurações" subtitle="segurança, banda e inicialização" />

<section class="grid grid-cols-2 gap-x-10 gap-y-8 border-t border-line-soft px-5 py-5">
  <div class="flex flex-col gap-3">
    <div class="sec-label accent">senha mestre</div>
    <p class="text-[12px] text-muted">
      Criptografa o <span class="text-fg">rclone.conf</span> (tokens e senhas das contas) com a criptografia nativa do
      rclone (NaCl secretbox: XSalsa20 + Poly1305). A chave vem de um hash rápido (SHA-256) da senha, então use uma
      senha longa — uma frase de várias palavras é ideal. Ao abrir o HSync, a senha será pedida.
    </p>
    <div class="flex items-center gap-2 text-[12.5px]">
      {#if encrypted}
        <Lock size={14} class="text-ok" /> <span class="text-ok">configuração criptografada</span>
      {:else}
        <LockOpen size={14} class="text-warn" /> <span class="text-warn">configuração sem senha</span>
      {/if}
    </div>
    {#if encrypted}
      <input class="input" type="password" placeholder="senha atual" bind:value={pwCurrent} autocomplete="current-password" />
    {/if}
    <input class="input" type="password" placeholder="nova senha (mín. 8 caracteres)" bind:value={pwNew} autocomplete="new-password" />
    <input class="input" type="password" placeholder="confirmar nova senha" bind:value={pwConfirm} autocomplete="new-password" />
    {#if pwError}<p class="text-[12px] text-err">✕ {pwError}</p>{/if}
    <div class="flex gap-2">
      <button class="btn btn-primary" onclick={savePassword} disabled={pwBusy || !pwNew || (encrypted && !pwCurrent)}>
        <KeyRound size={14} /> {encrypted ? "alterar senha" : "criptografar"}
      </button>
      {#if encrypted}
        <button class="btn btn-danger" onclick={removePassword} disabled={pwBusy || !pwCurrent}>remover criptografia</button>
      {/if}
    </div>
    <p class="text-[11px] text-warn">
      ⚠ não há recuperação: sem a senha, será preciso reconectar todas as contas.
    </p>
  </div>

  <div class="flex flex-col gap-8">
    <div class="flex flex-col gap-3">
      <div class="sec-label accent">limite de banda</div>
      <p class="text-[12px] text-muted">Vale para todas as transferências. Vazio = sem limite. Ex.: 512K, 2M, 1.5G (por segundo).</p>
      <div class="flex gap-2">
        <input class="input w-[160px]" bind:value={bw} placeholder="sem limite" onkeydown={(e) => e.key === "Enter" && saveBw()} />
        <button class="btn" onclick={saveBw}>aplicar</button>
      </div>
    </div>

    <div class="flex flex-col gap-3">
      <div class="sec-label accent">inicialização</div>
      <label class="flex cursor-pointer items-center gap-2 text-[12.5px]">
        <input type="checkbox" checked={autostart} onchange={toggleAutostart} class="accent-[#5cb8e4]" />
        iniciar com o sistema (minimizado na bandeja)
      </label>
      <label class="flex cursor-pointer items-center gap-2 text-[12.5px]">
        <input
          type="checkbox"
          checked={app.settings?.schedulesPaused ?? false}
          onchange={(e) => app.act(() => api.setSchedulesPaused(e.currentTarget.checked))}
          class="accent-[#5cb8e4]"
        />
        pausar todos os agendamentos
      </label>
      <p class="text-[11px] text-dim">fechar a janela mantém o HSync na bandeja; use "sair" na barra inferior para encerrar.</p>
    </div>

    <div class="flex flex-col gap-2">
      <div class="sec-label">sobre</div>
      <dl class="grid grid-cols-[120px_1fr] gap-y-1 text-[12px]">
        <dt class="text-muted">rclone</dt>
        <dd>{version}</dd>
        <dt class="text-muted">RC</dt>
        <dd>127.0.0.1, porta aleatória, credenciais efêmeras</dd>
        <dt class="text-muted">privacidade</dt>
        <dd>sem servidores intermediários</dd>
      </dl>
    </div>
  </div>
</section>
