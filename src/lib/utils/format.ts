import type { RunStatus, SyncMode } from "../api/types";

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(n: number | null | undefined): string {
  if (!n || n < 0) return "0 B";
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${n} B` : `${v.toLocaleString("pt-BR", { maximumFractionDigits: 1, minimumFractionDigits: 1 })} ${UNITS[i]}`;
}

export function formatSpeed(bytesPerSec: number): string {
  return `${formatBytes(Math.round(bytesPerSec))}/s`;
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !isFinite(seconds) || seconds < 0) return "—";
  const s = Math.round(seconds);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}min ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, "0")}min`;
}

export function formatDateTime(ms: number | null | undefined): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString("pt-BR", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
}

/** "em 12 min", "há 3 h" etc. */
export function formatRelative(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return "—";
  const diff = ms - now;
  const abs = Math.abs(diff);
  const min = Math.round(abs / 60_000);
  let txt: string;
  if (min < 1) txt = "menos de 1 min";
  else if (min < 60) txt = `${min} min`;
  else if (min < 48 * 60) txt = `${Math.round(min / 60)} h`;
  else txt = `${Math.round(min / 1440)} dias`;
  if (min < 1 && diff <= 0) return "agora";
  return diff > 0 ? `em ${txt}` : `há ${txt}`;
}

export function formatInterval(minutes: number): string {
  if (minutes % 1440 === 0) return minutes === 1440 ? "1 dia" : `${minutes / 1440} dias`;
  if (minutes % 60 === 0) return minutes === 60 ? "1 hora" : `${minutes / 60} horas`;
  return `${minutes} min`;
}

export const MODE_LABEL: Record<SyncMode, string> = {
  copy: "cópia",
  sync: "espelho",
  bisync: "bidirecional",
};

export const MODE_HELP: Record<SyncMode, string> = {
  copy: "Envia arquivos novos e alterados. Nunca apaga nada no destino.",
  sync: "Deixa o destino idêntico à origem — apaga do destino o que não existe na origem.",
  bisync: "Mantém as duas pontas iguais: mudanças de qualquer lado são propagadas.",
};

export const STATUS_LABEL: Record<RunStatus, string> = {
  running: "executando",
  success: "concluída",
  failed: "falhou",
  cancelled: "cancelada",
  interrupted: "interrompida",
};

export const STATUS_COLOR: Record<RunStatus, string> = {
  running: "text-accent",
  success: "text-ok",
  failed: "text-err",
  cancelled: "text-muted",
  interrupted: "text-warn",
};

export function isLocalPath(p: string): boolean {
  return /^[A-Za-z]:([\\/]|$)/.test(p) || p.startsWith("\\\\") || p.startsWith("/");
}

/** Nome da conta de um caminho `conta:pasta`, ou null para caminhos locais. */
export function remoteOf(p: string): string | null {
  if (!p || isLocalPath(p)) return null;
  const i = p.indexOf(":");
  return i > 0 ? p.slice(0, i) : null;
}

export function directionOf(source: string, dest: string): string {
  const s = isLocalPath(source);
  const d = isLocalPath(dest);
  if (s && !d) return "upload";
  if (!s && d) return "download";
  if (s && d) return "local ↔ local";
  return "nuvem → nuvem";
}

export const PROVIDER_LABEL: Record<string, string> = {
  drive: "Google Drive",
  onedrive: "OneDrive",
  mega: "Mega",
  dropbox: "Dropbox",
  s3: "S3",
  b2: "Backblaze B2",
  box: "Box",
  pcloud: "pCloud",
  webdav: "WebDAV",
  sftp: "SFTP",
  ftp: "FTP",
  local: "Local",
};

export function providerLabel(kind: string): string {
  return PROVIDER_LABEL[kind] ?? kind;
}
