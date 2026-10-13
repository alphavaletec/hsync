<script lang="ts">
  // Pré-visualização de um arquivo da aba Arquivos, com navegação ← → entre os arquivos da pasta.
  import { ChevronLeft, ChevronRight, ExternalLink, FileQuestion, FolderOpen, LoaderCircle, X } from "@lucide/svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, errorMessage } from "../../api/commands";
  import type { FileEntry, Preview } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import { formatBytes } from "../../utils/format";

  let {
    remote,
    files,
    index = $bindable(),
    onclose,
  }: { remote: string; files: FileEntry[]; index: number; onclose: () => void } = $props();

  let preview = $state<Preview | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let wrap = $state(true);

  const entry = $derived(files[index]);
  const src = $derived(preview ? convertFileSrc(preview.localPath) : "");
  const lines = $derived(preview?.text?.split(/\r?\n/) ?? []);

  // Carrega a cada troca de arquivo; descarta respostas de arquivos já deixados para trás.
  $effect(() => {
    const e = entry;
    if (!e) return;
    preview = null;
    error = null;
    loading = true;
    api
      .previewFile(remote, e.path, e.inCloud ? e.size : (e.localSize ?? e.size), e.modTime)
      .then((p) => {
        if (files[index] === e) preview = p;
      })
      .catch((err) => {
        if (files[index] === e) error = errorMessage(err);
      })
      .finally(() => {
        if (files[index] === e) loading = false;
      });
  });

  function go(delta: number) {
    const n = index + delta;
    if (n >= 0 && n < files.length) index = n;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
    else if (e.key === "ArrowLeft") go(-1);
    else if (e.key === "ArrowRight") go(1);
  }
</script>

<svelte:window {onkeydown} />

<div class="fixed inset-0 z-40 flex items-center justify-center bg-black/70 p-6" role="presentation">
  <div class="term flex h-full max-h-[880px] w-full max-w-[1200px] flex-col" role="dialog" aria-modal="true" aria-label="Pré-visualização">
    <div class="term-bar shrink-0">
      <span class="dots"><i></i><i></i><i></i></span>
      <button class="btn btn-ghost btn-icon h-6 w-6" onclick={() => go(-1)} disabled={index === 0} title="Anterior (←)"><ChevronLeft size={14} /></button>
      <span class="min-w-0 flex-1 truncate text-center text-fg" title={entry?.path}>
        {remote}:/{entry?.path}
      </span>
      <span class="text-dim">{index + 1}/{files.length}</span>
      <button class="btn btn-ghost btn-icon h-6 w-6" onclick={() => go(1)} disabled={index >= files.length - 1} title="Próximo (→)"><ChevronRight size={14} /></button>
      <button class="btn btn-ghost btn-icon h-6 w-6" onclick={onclose} aria-label="Fechar (Esc)"><X size={14} /></button>
    </div>

    <div class="relative min-h-0 flex-1 overflow-hidden bg-[#0b0c0e]">
      {#if loading}
        <div class="flex h-full flex-col items-center justify-center gap-3 text-muted">
          <LoaderCircle size={22} class="spin text-accent" />
          <span class="text-[12.5px]">{entry?.local ? "abrindo cópia local…" : `baixando de ${remote}: para pré-visualizar…`}</span>
          <span class="text-[11px] text-dim">{formatBytes(entry?.size)}</span>
        </div>
      {:else if error}
        <div class="flex h-full flex-col items-center justify-center gap-2 px-10 text-center">
          <span class="text-err">✕</span>
          <p class="selectable max-w-[560px] text-[12.5px] text-err">{error}</p>
        </div>
      {:else if preview}
        {#if preview.kind === "image"}
          <div class="checker flex h-full items-center justify-center overflow-auto p-4">
            <img {src} alt={preview.name} class="max-h-full max-w-full object-contain" />
          </div>
        {:else if preview.kind === "pdf"}
          <iframe {src} title={preview.name} class="h-full w-full border-0 bg-white"></iframe>
        {:else if preview.kind === "video"}
          <div class="flex h-full items-center justify-center p-4">
            <!-- svelte-ignore a11y_media_has_caption -->
            <video {src} controls class="max-h-full max-w-full"></video>
          </div>
        {:else if preview.kind === "audio"}
          <div class="flex h-full flex-col items-center justify-center gap-4">
            <span class="text-[14px]">{preview.name}</span>
            <audio {src} controls class="w-[420px]"></audio>
          </div>
        {:else if preview.kind === "text"}
          <div class="selectable h-full overflow-auto py-3 text-[12px] leading-[1.6]">
            <table class="border-collapse">
              <tbody>
                {#each lines as line, i (i)}
                  <tr>
                    <td class="select-none pr-4 pl-4 text-right align-top text-dim">{i + 1}</td>
                    <td class="pr-6 text-fg/90 {wrap ? 'break-all whitespace-pre-wrap' : 'whitespace-pre'}">{line || " "}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
            {#if preview.truncated}
              <p class="px-4 pt-3 text-[11.5px] text-warn">◇ mostrando apenas o primeiro 1 MB do arquivo</p>
            {/if}
          </div>
        {:else}
          <div class="flex h-full flex-col items-center justify-center gap-3 text-center">
            <FileQuestion size={36} class="text-dim" />
            <p class="text-[13px]">sem pré-visualização para este tipo de arquivo</p>
            {#if preview.canOpenExternal}
              <p class="text-[11.5px] text-muted">abra no aplicativo instalado (Word, Excel, etc.)</p>
            {:else}
              <p class="text-[11.5px] text-muted">por segurança, o HSync não abre este tipo de arquivo</p>
            {/if}
          </div>
        {/if}
      {/if}
    </div>

    <div class="flex shrink-0 items-center gap-2 border-t border-line px-4 py-2 text-[11.5px]">
      {#if preview}
        <span class="text-muted">{preview.name} · {formatBytes(preview.size)}</span>
        <span class="text-dim">· {preview.fromLocalCopy ? "lido da cópia local" : "cache de pré-visualização"}</span>
        {#if preview.kind === "text"}
          <label class="ml-4 flex cursor-pointer items-center gap-1.5 text-muted">
            <input type="checkbox" bind:checked={wrap} class="accent-[#5cb8e4]" /> quebrar linhas
          </label>
        {/if}
        <div class="ml-auto flex gap-2">
          <button class="btn h-7" onclick={() => revealItemInDir(preview!.localPath)}><FolderOpen size={13} /> mostrar arquivo</button>
          {#if preview.canOpenExternal}
            <button class="btn btn-primary h-7" onclick={() => app.act(() => api.openPreviewExternal(preview!.localPath))}>
              <ExternalLink size={13} /> abrir no aplicativo
            </button>
          {/if}
        </div>
      {:else}
        <span class="text-dim">← → navegar · Esc fechar</span>
      {/if}
    </div>
  </div>
</div>

<style>
  .checker {
    background-color: #141619;
    background-image:
      linear-gradient(45deg, #1b1e22 25%, transparent 25%),
      linear-gradient(-45deg, #1b1e22 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #1b1e22 75%),
      linear-gradient(-45deg, transparent 75%, #1b1e22 75%);
    background-size: 20px 20px;
    background-position: 0 0, 0 10px, 10px -10px, -10px 0;
  }
</style>
