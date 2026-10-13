// Estado global do app, alimentado por comandos iniciais + eventos do backend.
import { api, errorMessage } from "../api/commands";
import { on } from "../api/events";
import type { DaemonStatus, Job, JobProgress, LogLine, Remote, RunRecord, Settings } from "../api/types";
import { toasts } from "./toasts.svelte";

export type View = "dashboard" | "files" | "jobs" | "accounts" | "settings";

const LOG_LIMIT = 500;
const SPEED_SAMPLES = 90;

class AppStore {
  view = $state<View>("dashboard");
  /** Conta aberta na aba Arquivos. */
  filesRemote = $state<string | null>(null);
  daemon = $state<DaemonStatus>({ state: "starting" });
  jobs = $state<Job[]>([]);
  remotes = $state<Remote[]>([]);
  runs = $state<RunRecord[]>([]);
  progress = $state<JobProgress[]>([]);
  /** Velocidade agregada (bytes/s), uma amostra por segundo. */
  speedHistory = $state<number[]>([]);
  logs = $state<LogLine[]>([]);
  settings = $state<Settings | null>(null);

  ready = $derived(this.daemon.state === "ready");
  totalSpeed = $derived(this.progress.reduce((a, p) => a + p.speed, 0));

  #started = false;

  async init() {
    if (this.#started) return;
    this.#started = true;

    await Promise.all([
      on("daemon-status", (s) => {
        const wasReady = this.ready;
        this.daemon = s;
        if (s.state === "ready" && !wasReady) void this.refreshAll();
      }),
      on("daemon-log", (lines) => {
        const next = this.logs.concat(lines);
        this.logs = next.length > LOG_LIMIT ? next.slice(next.length - LOG_LIMIT) : next;
      }),
      on("job-progress", (p) => {
        this.progress = p;
        this.pushSpeed(p.reduce((a, x) => a + x.speed, 0));
      }),
      on("job-finished", (f) => {
        const what = f.dryRun ? `Simulação de "${f.jobName}"` : `"${f.jobName}"`;
        if (f.status === "success") toasts.push("ok", `${what} concluída`);
        else if (f.status === "cancelled") toasts.push("info", `${what} cancelada`);
        else toasts.push("err", `${what}: ${f.error ?? "falhou"}`);
        void this.refreshRuns();
      }),
      on("jobs-changed", () => {
        void this.refreshJobs();
        void this.refreshSettings();
      }),
    ]);

    this.daemon = await api.daemonStatus();
    this.logs = await api.daemonLogs();
    this.progress = await api.getProgress();
    void this.refreshSettings();
    if (this.ready) await this.refreshAll();
  }

  pushSpeed(v: number) {
    const next = this.speedHistory.concat(v);
    this.speedHistory = next.length > SPEED_SAMPLES ? next.slice(next.length - SPEED_SAMPLES) : next;
  }

  async refreshAll() {
    await Promise.all([this.refreshJobs(), this.refreshRemotes(), this.refreshRuns(), this.refreshSettings()]);
  }

  async refreshJobs() {
    try {
      this.jobs = await api.listJobs();
    } catch (e) {
      console.warn(errorMessage(e));
    }
  }

  async refreshRemotes() {
    if (!this.ready) return;
    try {
      this.remotes = await api.listRemotes();
    } catch (e) {
      console.warn(errorMessage(e));
    }
  }

  async refreshRuns() {
    try {
      this.runs = await api.listRuns(undefined, 100);
    } catch (e) {
      console.warn(errorMessage(e));
    }
  }

  async refreshSettings() {
    try {
      this.settings = await api.getSettings();
    } catch (e) {
      console.warn(errorMessage(e));
    }
  }

  /** Executa uma ação do usuário mostrando erro como toast. */
  async act<T>(fn: () => Promise<T>, success?: string): Promise<T | undefined> {
    try {
      const r = await fn();
      if (success) toasts.push("ok", success);
      return r;
    } catch (e) {
      toasts.push("err", errorMessage(e));
      return undefined;
    }
  }
}

export const app = new AppStore();
