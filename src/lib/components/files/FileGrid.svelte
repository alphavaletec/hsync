<script module lang="ts">
  /** Miniaturas já resolvidas (sobrevive a trocar de pasta/lista e voltar): caminho da imagem ou `null`. */
  const resolved = new Map<string, string | null>();
</script>

<script lang="ts">
  // Visualização em grade da aba Arquivos: miniaturas das imagens (geradas só para o que está
  // visível na tela), pastas e demais arquivos como ícones.
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { Cloud, File, FileText, Folder, HardDrive, Image as ImageIcon, ImageOff, LoaderCircle } from "@lucide/svelte";
  import { api } from "../../api/commands";
  import type { FileEntry, ItemState } from "../../api/types";
  import { ITEM_STATE, formatBytes } from "../../utils/format";

  type Effective = { state: ItemState; explicit: boolean; from?: string } | null;

  let {
    remote,
    entries,
    selected,
    states,
    effective,
    busyPath,
    ontoggle,
    onopen,
    onpreview,
    onstate,
  }: {
    remote: string;
    entries: FileEntry[];
    selected: Set<string>;
    states: ItemState[];
    effective: (path: string) => Effective;
    busyPath: string | null;
    ontoggle: (path: string) => void;
    onopen: (path: string) => void;
    onpreview: (e: FileEntry) => void;
    onstate: (e: FileEntry, value: string) => void;
  } = $props();

  const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp)$/i;
  const TEXT_EXT = /\.(txt|md|pdf|docx?|xlsx?|pptx?|csv|odt|ods|rtf)$/i;

  /** Miniaturas já pedidas: caminho da imagem pronta, `null` (sem miniatura) ou "loading". */
  let thumbs = $state<Record<string, string | null | "loading">>({});

  /** Acima disso o app não gera miniatura (o backend também recusa). */
  const MAX_THUMB_SOURCE = 25 * 1024 * 1024;
  const keyOf = (e: FileEntry) => `${remote}|${e.path}|${e.size}|${e.modTime ?? ""}`;
  const isImage = (e: FileEntry) => !e.isDir && IMAGE_EXT.test(e.name);
  const thumbOf = (e: FileEntry) => thumbs[keyOf(e)] ?? resolved.get(keyOf(e));

  async function load(e: FileEntry) {
    const key = keyOf(e);
    if (thumbOf(e) !== undefined) return;
    if (e.size > MAX_THUMB_SOURCE) {
      resolved.set(key, null);
      thumbs[key] = null;
      return;
    }
    thumbs[key] = "loading";
    let result: string | null = null;
    try {
      result = await api.thumbnail(remote, e.path, e.size, e.modTime);
    } catch {
      /* sem miniatura: fica o ícone */
    }
    resolved.set(key, result);
    thumbs[key] = result;
  }

  /** Pede a miniatura quando o cartão aparece na tela (e só uma vez). */
  function lazy(node: HTMLElement, e: FileEntry) {
    if (!isImage(e)) return;
    if (typeof IntersectionObserver === "undefined") {
      void load(e);
      return;
    }
    const io = new IntersectionObserver(
      (items) => {
        if (items.some((i) => i.isIntersecting)) {
          io.disconnect();
          void load(e);
        }
      },
      { rootMargin: "300px" },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  function srcOf(e: FileEntry): string | null {
    const t = thumbOf(e);
    return t && t !== "loading" ? convertFileSrc(t) : null;
  }
</script>

<div class="grid grid-cols-[repeat(auto-fill,minmax(148px,1fr))] gap-3 p-3">
  {#each entries as e (e.path)}
    {@const eff = effective(e.path)}
    {@const src = srcOf(e)}
    {@const t = thumbOf(e)}
    <div
      class="group relative flex flex-col border bg-node/40 transition-colors hover:border-accent-dim {selected.has(e.path)
        ? 'border-accent bg-accent/5'
        : 'border-line'}"
    >
      <button
        class="relative flex aspect-square w-full items-center justify-center overflow-hidden bg-[#101214]"
        use:lazy={e}
        onclick={() => (e.isDir ? onopen(e.path) : onpreview(e))}
        title={e.isDir ? `Abrir ${e.name}` : `${e.name} — pré-visualizar`}
        aria-label={e.isDir ? `Abrir ${e.name}` : `Pré-visualizar ${e.name}`}
      >
        {#if e.isDir}
          <Folder size={44} class="text-accent/80" />
        {:else if src}
          <img {src} alt={e.name} loading="lazy" draggable="false" class="h-full w-full object-cover" />
        {:else if t === "loading"}
          <LoaderCircle size={20} class="spin text-dim" />
        {:else if isImage(e) && t === null}
          <ImageOff size={30} class="text-dim" title="Sem miniatura" />
        {:else if isImage(e)}
          <ImageIcon size={30} class="text-dim/60" />
        {:else if TEXT_EXT.test(e.name)}
          <FileText size={34} class="text-muted" />
        {:else}
          <File size={34} class="text-muted" />
        {/if}
      </button>

      <input
        type="checkbox"
        class="absolute top-1.5 left-1.5 accent-[#5cb8e4] {selected.has(e.path) ? '' : 'opacity-0 group-hover:opacity-100 focus:opacity-100'}"
        checked={selected.has(e.path)}
        onchange={() => ontoggle(e.path)}
        aria-label="Selecionar {e.name}"
      />
      {#if eff}
        <span
          class="absolute top-1 right-1 bg-[#0c0e10]/80 px-1 text-[12px] {ITEM_STATE[eff.state].cls}"
          title={eff.explicit ? ITEM_STATE[eff.state].label : `herdado de ${eff.from}: ${ITEM_STATE[eff.state].label}`}
        >
          {ITEM_STATE[eff.state].icon}
        </span>
      {/if}

      <div class="flex min-w-0 flex-col gap-1 px-2 py-1.5">
        <span class="truncate text-[12px]" title={e.name}>{e.name}</span>
        <span class="flex items-center gap-1.5 text-[10.5px] text-dim">
          {#if !e.isDir}<span>{formatBytes(e.size)}</span>{/if}
          {#if e.inCloud && e.local}
            <span class="text-ok" title="Na nuvem e neste computador">◆</span>
          {:else if e.inCloud}
            <Cloud size={11} title="Só na nuvem" />
          {:else}
            <HardDrive size={11} class="text-warn" title="Só neste computador" />
          {/if}
          {#if busyPath === e.path}<LoaderCircle size={11} class="spin ml-auto text-accent" />{/if}
        </span>
        <select
          class="input h-6 py-0 text-[11px] {eff ? ITEM_STATE[eff.state].cls : 'text-dim'} {eff && !eff.explicit ? 'opacity-70' : ''}"
          value={eff?.explicit ? eff.state : ""}
          disabled={busyPath !== null}
          onchange={(ev) => onstate(e, ev.currentTarget.value)}
          title={eff && !eff.explicit ? `herdado de ${eff.from}` : eff ? ITEM_STATE[eff.state].help : "não sincronizado"}
          aria-label="Estado de {e.name}"
        >
          <option value="">{eff && !eff.explicit ? `${ITEM_STATE[eff.state].icon} herdado` : "— nenhum"}</option>
          {#each states as s (s)}
            <option value={s}>{ITEM_STATE[s].icon} {ITEM_STATE[s].label}</option>
          {/each}
        </select>
      </div>
    </div>
  {/each}
</div>
