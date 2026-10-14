// Atualização automática: consulta a página de lançamentos do GitHub (latest.json), baixa a
// versão nova e a instala depois que o usuário confirma. A assinatura é conferida pelo plugin.
import { getVersion } from "@tauri-apps/api/app";
import { ask } from "@tauri-apps/plugin-dialog";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { errorMessage } from "../api/commands";
import { app } from "./app.svelte";

const FIRST_CHECK_MS = 20_000;
const EVERY_MS = 6 * 60 * 60 * 1000;

type Status = "idle" | "checking" | "uptodate" | "available" | "downloading" | "installing" | "error";

class UpdaterStore {
  status = $state<Status>("idle");
  current = $state("");
  version = $state<string | null>(null);
  notes = $state<string | null>(null);
  /** 0–100; null enquanto o tamanho é desconhecido. */
  progress = $state<number | null>(null);
  error = $state<string | null>(null);
  /** O usuário dispensou o aviso desta versão (volta a aparecer em uma versão mais nova). */
  dismissedVersion = $state<string | null>(null);

  #update: Update | null = null;

  get banner(): boolean {
    return (
      (this.status === "available" && this.version !== this.dismissedVersion) ||
      this.status === "downloading" ||
      this.status === "installing"
    );
  }

  /** Verificação automática periódica (só no app instalado: em desenvolvimento não há o que atualizar). */
  start() {
    getVersion().then((v) => (this.current = v)).catch(() => {});
    if (import.meta.env.DEV) return;
    const tick = () => {
      void this.check(false).finally(() => setTimeout(tick, EVERY_MS));
    };
    setTimeout(tick, FIRST_CHECK_MS);
  }

  /** `manual`: mostra o resultado (inclusive erros); a automática falha em silêncio (sem internet etc.). */
  async check(manual: boolean) {
    if (["checking", "downloading", "installing"].includes(this.status)) return;
    const before = this.status;
    this.status = "checking";
    this.error = null;
    try {
      const update = await check();
      if (update) {
        this.#update = update;
        this.version = update.version;
        this.notes = update.body ?? null;
        this.status = "available";
      } else {
        this.#update = null;
        this.status = "uptodate";
      }
    } catch (e) {
      this.status = manual ? "error" : before === "available" ? "available" : "idle";
      if (manual) this.error = errorMessage(e);
      else console.warn("verificação de atualização:", errorMessage(e));
    }
  }

  async install() {
    const update = this.#update;
    if (!update || this.status !== "available") return;
    if (app.progress.length > 0) {
      const ok = await ask(`${app.progress.length} tarefa(s) em execução serão interrompidas para atualizar. Continuar?`, {
        title: "Atualizar",
        kind: "warning",
        okLabel: "Atualizar",
        cancelLabel: "Depois",
      });
      if (!ok) return;
    }
    this.status = "downloading";
    this.progress = null;
    let total = 0;
    let received = 0;
    try {
      // No Windows o instalador fecha o app, instala e abre a versão nova sozinho.
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") {
          received += ev.data.chunkLength;
          this.progress = total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null;
        } else if (ev.event === "Finished") this.status = "installing";
      });
      this.status = "installing";
    } catch (e) {
      this.status = "error";
      this.error = errorMessage(e);
    }
  }

  dismiss() {
    this.dismissedVersion = this.version;
  }
}

export const updater = new UpdaterStore();
