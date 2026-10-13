<script lang="ts">
  // Renderiza Word (.docx), planilhas (.xlsx/.xls/.ods/.csv) e PowerPoint (.pptx) localmente,
  // sem serviços online. As bibliotecas só são carregadas quando um arquivo do tipo é aberto.
  import { LoaderCircle } from "@lucide/svelte";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import type { PreviewKind } from "../../api/types";
  import { errorMessage } from "../../api/commands";

  let { kind, localPath }: { kind: PreviewKind; localPath: string } = $props();

  const MAX_ROWS = 2000;
  const MAX_COLS = 60;

  let host: HTMLDivElement | undefined = $state();
  let styles: HTMLDivElement | undefined = $state();
  let loading = $state(true);
  let error = $state<string | null>(null);
  let note = $state<string | null>(null);

  // Planilha
  let sheetNames = $state<string[]>([]);
  let activeSheet = $state(0);
  let rows = $state<string[][]>([]);
  let workbook: import("xlsx").WorkBook | null = null;
  let xlsx: typeof import("xlsx") | null = null;

  let disposeViewer: (() => void) | null = null;
  onDestroy(() => disposeViewer?.());

  async function bytes(): Promise<ArrayBuffer> {
    const resp = await fetch(convertFileSrc(localPath));
    if (!resp.ok) throw new Error(`falha ao ler o arquivo (${resp.status})`);
    return resp.arrayBuffer();
  }

  function colName(i: number): string {
    let s = "";
    for (let n = i + 1; n > 0; n = Math.floor((n - 1) / 26)) s = String.fromCharCode(65 + ((n - 1) % 26)) + s;
    return s;
  }

  function showSheet(i: number) {
    if (!workbook || !xlsx) return;
    activeSheet = i;
    const ws = workbook.Sheets[workbook.SheetNames[i]];
    const all = xlsx.utils.sheet_to_json<unknown[]>(ws, { header: 1, raw: false, defval: "", blankrows: true, dateNF: "dd/mm/yyyy" });
    const width = Math.min(MAX_COLS, Math.max(0, ...all.map((r) => r.length)));
    rows = all.slice(0, MAX_ROWS).map((r) => Array.from({ length: width }, (_, c) => String(r[c] ?? "")));
    const cut: string[] = [];
    if (all.length > MAX_ROWS) cut.push(`${MAX_ROWS} de ${all.length} linhas`);
    if (all.some((r) => r.length > MAX_COLS)) cut.push(`${MAX_COLS} colunas`);
    note = cut.length ? `mostrando ${cut.join(" e ")} — abra no aplicativo para ver tudo` : null;
  }

  /** Documentos podem conter links: nunca navegar o webview para fora do app. */
  function blockLinks(e: MouseEvent) {
    if ((e.target as HTMLElement | null)?.closest("a")) e.preventDefault();
  }

  $effect(() => {
    // Word/PowerPoint desenham num contêiner; a planilha é renderizada pelo próprio Svelte.
    const target = host;
    if (kind !== "sheet" && !target) return;
    let cancelled = false;
    (async () => {
      try {
        const data = await bytes();
        if (cancelled) return;
        if (kind === "docx") {
          const { renderAsync } = await import("docx-preview");
          await renderAsync(data, target!, styles, {
            className: "docx",
            inWrapper: true,
            breakPages: true,
            ignoreLastRenderedPageBreak: true,
            renderHeaders: true,
            renderFooters: true,
            renderFootnotes: true,
            useBase64URL: true,
            experimental: false,
          });
        } else if (kind === "slides") {
          const { PptxViewer, RECOMMENDED_ZIP_LIMITS } = await import("@aiden0z/pptx-renderer");
          if (cancelled) return;
          const viewer = await PptxViewer.open(data, target!, {
            zipLimits: RECOMMENDED_ZIP_LIMITS,
            lazySlides: true,
            lazyMedia: true,
            fitMode: "contain",
            listOptions: { windowed: true, initialSlides: 3, batchSize: 3, showSlideLabels: true },
          });
          disposeViewer = () => viewer.destroy();
          note = `${viewer.slideCount} slides · fidelidade aproximada (fontes e efeitos podem diferir)`;
        } else {
          xlsx = await import("xlsx");
          workbook = xlsx.read(data, { type: "array", cellDates: true, dense: true });
          sheetNames = workbook.SheetNames;
          showSheet(0);
        }
      } catch (e) {
        if (!cancelled) error = errorMessage(e);
      } finally {
        if (!cancelled) loading = false;
      }
    })();
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="flex h-full flex-col">
  {#if note}
    <div class="shrink-0 border-b border-line bg-[#15171a] px-4 py-1.5 text-[11px] text-muted">◇ {note}</div>
  {/if}

  <div class="relative min-h-0 flex-1">
    {#if loading}
      <div class="absolute inset-0 z-10 flex items-center justify-center gap-2 text-[12.5px] text-muted">
        <LoaderCircle size={16} class="spin text-accent" /> renderizando…
      </div>
    {/if}
    {#if error}
      <div class="flex h-full items-center justify-center px-10 text-center">
        <p class="selectable max-w-[560px] text-[12.5px] text-err">
          ✕ não foi possível renderizar este arquivo: {error}<br />
          <span class="text-muted">use "abrir no aplicativo".</span>
        </p>
      </div>
    {/if}

    {#if kind === "sheet"}
      <div class="selectable h-full overflow-auto">
        {#if rows.length}
          <table class="sheet">
            <thead>
              <tr>
                <th></th>
                {#each rows[0] as _, c (c)}<th>{colName(c)}</th>{/each}
              </tr>
            </thead>
            <tbody>
              {#each rows as row, r (r)}
                <tr>
                  <th>{r + 1}</th>
                  {#each row as cell, c (c)}<td title={cell.length > 40 ? cell : undefined}>{cell}</td>{/each}
                </tr>
              {/each}
            </tbody>
          </table>
        {:else if !loading && !error}
          <p class="p-4 text-[12px] text-dim">◇ planilha vazia</p>
        {/if}
      </div>
    {:else}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="office-host h-full overflow-auto" class:slides={kind === "slides"} onclickcapture={blockLinks}>
        <div bind:this={styles}></div>
        <div bind:this={host}></div>
      </div>
    {/if}
  </div>

  {#if kind === "sheet" && sheetNames.length > 1}
    <div class="flex shrink-0 gap-px overflow-x-auto border-t border-line bg-[#15171a] text-[11.5px]">
      {#each sheetNames as name, i (i)}
        <button
          class="shrink-0 px-3 py-1.5 {i === activeSheet ? 'bg-node text-fg' : 'text-muted hover:text-fg'}"
          onclick={() => showSheet(i)}
        >
          {name}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .sheet {
    border-collapse: collapse;
    font-size: 12px;
    color: #1f2328;
    background: #fff;
    min-width: 100%;
  }
  .sheet th,
  .sheet td {
    border: 1px solid #d0d7de;
    padding: 2px 8px;
    white-space: nowrap;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sheet thead th,
  .sheet tbody th {
    position: sticky;
    background: #eef1f4;
    color: #57606a;
    font-weight: 500;
    text-align: center;
  }
  .sheet thead th {
    top: 0;
    z-index: 1;
  }
  .sheet tbody th {
    left: 0;
    min-width: 44px;
  }
  .sheet thead th:first-child {
    left: 0;
    z-index: 2;
  }
  /* Documentos sem fonte definida não devem herdar a fonte monoespaçada do app. */
  .office-host,
  .sheet {
    font-family: Calibri, Carlito, "Segoe UI", Arial, sans-serif;
  }
  /* O docx-preview desenha páginas brancas sobre um fundo cinza (classe docx-wrapper). */
  .office-host :global(.docx-wrapper) {
    background: #2a2d31;
    padding: 24px;
  }
  .office-host.slides {
    background: #2a2d31;
    padding: 16px;
  }
</style>
