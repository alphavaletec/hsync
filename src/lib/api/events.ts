import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DaemonStatus, JobFinished, JobProgress, LogLine } from "./types";

interface EventMap {
  "daemon-status": DaemonStatus;
  "daemon-log": LogLine[];
  "job-progress": JobProgress[];
  "job-finished": JobFinished;
  "jobs-changed": null;
}

export function on<K extends keyof EventMap>(event: K, handler: (payload: EventMap[K]) => void): Promise<UnlistenFn> {
  return listen<EventMap[K]>(event, (e) => handler(e.payload));
}
