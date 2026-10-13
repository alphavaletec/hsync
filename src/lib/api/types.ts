// Espelha as structs serializadas pelo backend Rust (serde camelCase).

export type DaemonStatus =
  | { state: "starting" }
  | { state: "locked" }
  | { state: "ready"; port: number; version: string }
  | { state: "restarting"; attempt: number; reason: string }
  | { state: "failed"; reason: string }
  | { state: "maintenance" }
  | { state: "stopped" };

export interface LogLine {
  time: string;
  level: string;
  msg: string;
}

export type SyncMode = "sync" | "copy" | "bisync";
export type RunStatus = "running" | "success" | "failed" | "cancelled" | "interrupted";

export interface Filters {
  excludeDirs: string[];
  excludeExts: string[];
  excludeHidden: boolean;
  customRules: string[];
}

export interface Schedule {
  enabled: boolean;
  intervalMinutes: number;
}

export interface RunRecord {
  id: number;
  jobId: number;
  startedAt: number;
  finishedAt: number | null;
  status: RunStatus;
  dryRun: boolean;
  trigger: string;
  bytes: number;
  files: number;
  deletes: number;
  errors: number;
  error: string | null;
}

export interface Job {
  id: number;
  name: string;
  source: string;
  dest: string;
  mode: SyncMode;
  filters: Filters;
  schedule: Schedule;
  bisyncInitialized: boolean;
  createdAt: number;
  updatedAt: number;
  managed: string | null;
  lastRun: RunRecord | null;
  running: boolean;
  nextRunAt: number | null;
}

export interface JobInput {
  name: string;
  source: string;
  dest: string;
  mode: SyncMode;
  filters: Filters;
  schedule: Schedule;
}

export interface RunOptions {
  dryRun?: boolean;
  resync?: boolean;
}

export interface Transferring {
  name: string;
  size: number;
  bytes: number;
  percentage: number;
  speed: number;
  speedAvg: number;
  eta: number | null;
}

export interface JobProgress {
  jobId: number;
  runId: number;
  jobName: string;
  mode: SyncMode;
  dryRun: boolean;
  startedAt: number;
  bytes: number;
  totalBytes: number;
  percentage: number;
  speed: number;
  eta: number | null;
  transfers: number;
  totalTransfers: number;
  checks: number;
  totalChecks: number;
  deletes: number;
  errors: number;
  lastError: string | null;
  elapsed: number;
  transferring: Transferring[];
}

export interface JobFinished {
  jobId: number;
  runId: number;
  jobName: string;
  status: RunStatus;
  dryRun: boolean;
  error: string | null;
}

export interface Remote {
  name: string;
  kind: string;
  hasCustomClient: boolean;
}

export interface ProviderOption {
  name: string;
  help: string;
  kind: string;
  required: boolean;
  isPassword: boolean;
  default: string;
  examples: { value: string; help: string }[];
}

export interface Provider {
  name: string;
  description: string;
  options: ProviderOption[];
}

/** Objeto `Option` cru do rclone para perguntas do wizard. */
export interface RcloneOption {
  Name: string;
  Help: string;
  Type: string;
  Default: unknown;
  DefaultStr: string;
  Required: boolean;
  IsPassword: boolean;
  Exclusive: boolean;
  Examples?: { Value: string; Help: string }[] | null;
}

export type ConfigStep =
  | { step: "done" }
  | { step: "question"; state: string; option: RcloneOption; error: string | null }
  | { step: "oAuth"; jobId: number };

export interface OAuthProgress {
  finished: boolean;
  authUrl: string | null;
  next: ConfigStep | null;
  error: string | null;
}

export interface RemoteTest {
  ok: boolean;
  error: string | null;
  info: Record<string, unknown> | null;
  about: { total?: number; used?: number; free?: number; trashed?: number } | null;
}

export interface DirEntry {
  name: string;
  path: string;
}

export interface Settings {
  bwlimit: string;
  schedulesPaused: boolean;
  encrypted: boolean;
}

// ---- aba Arquivos ----

export type ItemState = "sync" | "download" | "upload" | "ignore";

export interface Selection {
  path: string;
  isDir: boolean;
  state: ItemState;
}

export interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  modTime: string | null;
  inCloud: boolean;
  local: boolean;
  localSize: number | null;
  localModMs: number | null;
}

export interface FilesConfig {
  remote: string;
  base: string;
  baseIsDefault: boolean;
  intervalMinutes: number;
  selections: Selection[];
  jobs: Job[];
}

export type PreviewKind = "image" | "pdf" | "text" | "audio" | "video" | "docx" | "sheet" | "slides" | "other";

export interface Preview {
  kind: PreviewKind;
  name: string;
  localPath: string;
  size: number;
  text: string | null;
  truncated: boolean;
  canOpenExternal: boolean;
  fromLocalCopy: boolean;
}
