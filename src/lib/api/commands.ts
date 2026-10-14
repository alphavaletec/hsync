// Wrappers tipados dos comandos Tauri. O frontend nunca fala com o rclone diretamente.
import { invoke } from "@tauri-apps/api/core";
import type {
  ConfigStep,
  DaemonStatus,
  DirEntry,
  FileEntry,
  FilesConfig,
  ItemState,
  SetStateResult,
  Job,
  JobInput,
  JobProgress,
  LogLine,
  OAuthProgress,
  OnDemandStatus,
  Preview,
  Provider,
  Remote,
  RemoteTest,
  RunOptions,
  RunRecord,
  Settings,
} from "./types";

export const api = {
  // daemon / configurações
  daemonStatus: () => invoke<DaemonStatus>("daemon_status"),
  daemonLogs: () => invoke<LogLine[]>("daemon_logs"),
  daemonRetry: () => invoke<void>("daemon_retry"),
  unlockConfig: (password: string) => invoke<void>("unlock_config", { password }),
  getSettings: () => invoke<Settings>("get_settings"),
  setBwlimit: (rate: string) => invoke<string>("set_bwlimit", { rate }),
  setMasterPassword: (current: string | null, next: string) =>
    invoke<void>("set_master_password", { current, new: next }),
  removeMasterPassword: (current: string) => invoke<void>("remove_master_password", { current }),
  quitApp: () => invoke<void>("quit_app"),

  // contas
  listRemotes: () => invoke<Remote[]>("list_remotes"),
  listProviders: () => invoke<Provider[]>("list_providers"),
  createRemote: (name: string, kind: string, parameters: Record<string, string>) =>
    invoke<ConfigStep>("create_remote", { input: { name, kind, parameters } }),
  answerConfig: (name: string, kind: string, configState: string, result: string) =>
    invoke<ConfigStep>("answer_config", { name, kind, configState, result }),
  oauthProgress: (name: string, kind: string, jobId: number) =>
    invoke<OAuthProgress>("oauth_progress", { name, kind, jobId }),
  cancelRemoteConfig: (name: string) => invoke<void>("cancel_remote_config", { name }),
  renameRemote: (old: string, next: string) => invoke<void>("rename_remote", { old, new: next }),
  reauthorizeRemote: (name: string, kind: string) => invoke<ConfigStep>("reauthorize_remote", { name, kind }),
  testRemote: (name: string) => invoke<RemoteTest>("test_remote", { name }),
  deleteRemote: (name: string) => invoke<void>("delete_remote", { name }),
  listRemoteDirs: (remote: string, path: string) => invoke<DirEntry[]>("list_remote_dirs", { remote, path }),
  makeRemoteDir: (remote: string, path: string) => invoke<void>("make_remote_dir", { remote, path }),

  // arquivos
  filesConfig: (remote: string) => invoke<FilesConfig>("files_config", { remote }),
  browse: (remote: string, path: string) => invoke<FileEntry[]>("browse", { remote, path }),
  setItemState: (remote: string, path: string, isDir: boolean, itemState: ItemState | null) =>
    invoke<SetStateResult>("set_item_state", { remote, path, isDir, itemState }),
  setFilesBase: (remote: string, base: string) => invoke<FilesConfig>("set_files_base", { remote, base }),
  setFilesInterval: (remote: string, minutes: number, onChange = false) =>
    invoke<FilesConfig>("set_files_interval", { remote, minutes, onChange }),
  freeSpace: (remote: string, items: { path: string; isDir: boolean }[]) =>
    invoke<string[]>("free_space", { remote, items }),
  syncAccountNow: (remote: string) => invoke<void>("sync_account_now", { remote }),
  downloadItems: (remote: string, items: { path: string; isDir: boolean }[], target: string) =>
    invoke<number>("download_items", { remote, items, target }),
  uploadItems: (remote: string, dir: string, localPaths: string[]) =>
    invoke<number>("upload_items", { remote, dir, localPaths }),

  previewFile: (remote: string, path: string, size: number, modTime: string | null) =>
    invoke<Preview>("preview_file", { remote, path, size, modTime }),
  openPreviewExternal: (localPath: string) => invoke<void>("open_preview_external", { localPath }),
  /** Caminho da miniatura (JPEG pequeno) de uma imagem, ou null se o formato/tamanho não é suportado. */
  thumbnail: (remote: string, path: string, size: number, modTime: string | null) =>
    invoke<string | null>("thumbnail", { remote, path, size, modTime }),

  ondemandStatus: (remote: string) => invoke<OnDemandStatus>("ondemand_status", { remote }),
  enableOndemand: (remote: string) => invoke<OnDemandStatus>("enable_ondemand", { remote }),
  disableOndemand: (remote: string) => invoke<void>("disable_ondemand", { remote }),
  ondemandRefresh: (remote: string) => invoke<void>("ondemand_refresh", { remote }),

  // tarefas
  listJobs: () => invoke<Job[]>("list_jobs"),
  createJob: (input: JobInput) => invoke<Job>("create_job", { input }),
  updateJob: (id: number, input: JobInput) => invoke<Job>("update_job", { id, input }),
  deleteJob: (id: number) => invoke<void>("delete_job", { id }),
  runJob: (id: number, options?: RunOptions) => invoke<number>("run_job", { id, options }),
  stopJob: (id: number) => invoke<void>("stop_job", { id }),
  runAllJobs: () => invoke<void>("run_all_jobs"),
  listRuns: (jobId?: number, limit?: number) => invoke<RunRecord[]>("list_runs", { jobId, limit }),
  getProgress: () => invoke<JobProgress[]>("get_progress"),
  setSchedulesPaused: (paused: boolean) => invoke<void>("set_schedules_paused", { paused }),
};

/** Erros dos comandos chegam como string (ver `impl Serialize for AppError`). */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
