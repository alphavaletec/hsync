<script lang="ts">
  import { Activity, Cloud, LoaderCircle, Plus, Trash2 } from "@lucide/svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage } from "../../api/commands";
  import type { Remote, RemoteTest } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import { formatBytes, providerLabel, remoteOf } from "../../utils/format";
  import AccountWizard from "../accounts/AccountWizard.svelte";
  import ViewHeader from "../ui/ViewHeader.svelte";

  let wizard = $state(false);
  let tests = $state<Record<string, RemoteTest | "loading">>({});

  const usage = (name: string) => app.jobs.filter((j) => remoteOf(j.source) === name || remoteOf(j.dest) === name).length;

  async function testRemote(r: Remote) {
    tests[r.name] = "loading";
    try {
      tests[r.name] = await api.testRemote(r.name);
    } catch (e) {
      tests[r.name] = { ok: false, error: errorMessage(e), info: null, about: null };
    }
  }

  async function remove(r: Remote) {
    const ok = await ask(
      `Remover a conta "${r.name}" do HSync? As credenciais locais são apagadas; nada é removido da nuvem.`,
      { title: "Remover conta", kind: "warning", okLabel: "Remover", cancelLabel: "Cancelar" },
    );
    if (!ok) return;
    const done = await app.act(() => api.deleteRemote(r.name), "Conta removida");
    if (done !== undefined) await app.refreshRemotes();
  }
</script>

<ViewHeader
  num="03"
  crumb="contas"
  title={app.remotes.length ? `${app.remotes.length} ${app.remotes.length === 1 ? "conta conectada" : "contas conectadas"}` : "nenhuma conta conectada"}
  subtitle="direto do seu PC para a nuvem"
>
  {#snippet legend()}
    <span><span class="text-ok">◆</span> testada ok</span>
    <span><span class="text-err">✕</span> falhou</span>
  {/snippet}
  {#snippet actions()}
    <button class="btn btn-primary" onclick={() => (wizard = true)} disabled={!app.ready}><Plus size={14} /> conectar conta</button>
  {/snippet}
</ViewHeader>

<section class="border-t border-line-soft px-7 py-5">
  {#if app.remotes.length === 0}
    <div class="term max-w-[620px] px-5 py-6 text-[12.5px] leading-[1.9]">
      <p><span class="text-ok">◆</span> Google Drive, OneDrive e Mega com assistente</p>
      <p><span class="text-ok">◆</span> mais de 50 provedores do rclone (S3, Dropbox, WebDAV, SFTP…)</p>
      <p class="text-dim">◇ nenhum servidor intermediário: tokens ficam no rclone.conf local</p>
    </div>
  {/if}

  <div class="grid grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-6">
    {#each app.remotes as r (r.name)}
      {@const t = tests[r.name]}
      <article class="node flex flex-col px-4 py-4">
        <span class="node-dot"></span>
        <div class="flex items-center gap-3">
          <Cloud size={20} class="text-accent" />
          <div class="min-w-0">
            <div class="truncate text-[14px] font-semibold">{r.name}:</div>
            <div class="text-[11px] text-muted">{providerLabel(r.kind)}</div>
          </div>
          {#if t && t !== "loading"}
            <span class="ml-auto {t.ok ? 'text-ok' : 'text-err'}">{t.ok ? "◆" : "✕"}</span>
          {/if}
        </div>

        <div class="mt-3 flex flex-wrap gap-2 text-[11px] text-muted">
          <span>{usage(r.name)} tarefa(s)</span>
          {#if r.hasCustomClient}<span class="text-ok">· client_id próprio</span>{/if}
          {#if r.kind === "drive" && !r.hasCustomClient}
            <span class="text-warn" title="O client_id compartilhado do rclone para o Drive está sendo descontinuado">· client_id compartilhado</span>
          {/if}
        </div>

        {#if t === "loading"}
          <div class="mt-3 flex items-center gap-2 text-[11.5px] text-muted"><LoaderCircle size={12} class="spin" /> testando…</div>
        {:else if t}
          {#if t.ok && t.about?.total}
            <div class="mt-3 text-[11.5px] text-muted">
              {formatBytes(t.about.used ?? 0)} / {formatBytes(t.about.total)}
            </div>
            <div class="progress mt-1"><div style:width="{((t.about.used ?? 0) / t.about.total) * 100}%"></div></div>
          {:else if t.ok}
            <div class="mt-3 text-[11.5px] text-ok">conexão ok</div>
          {:else}
            <div class="selectable mt-3 line-clamp-3 text-[11.5px] text-err" title={t.error ?? ""}>{t.error}</div>
          {/if}
        {/if}

        <div class="mt-4 flex gap-2">
          <button class="btn" onclick={() => testRemote(r)} disabled={t === "loading"}><Activity size={13} /> testar</button>
          <button class="btn btn-icon btn-danger ml-auto" onclick={() => remove(r)} title="Remover"><Trash2 size={14} /></button>
        </div>
      </article>
    {/each}
  </div>
</section>

{#if wizard}
  <AccountWizard onclose={() => ((wizard = false), app.refreshRemotes())} />
{/if}
