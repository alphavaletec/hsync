export type ToastKind = "ok" | "err" | "info";

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

class ToastStore {
  items = $state<Toast[]>([]);
  #next = 1;

  push(kind: ToastKind, text: string, ms = kind === "err" ? 8000 : 4000) {
    const id = this.#next++;
    this.items = [...this.items.slice(-4), { id, kind, text }];
    setTimeout(() => this.dismiss(id), ms);
  }

  dismiss(id: number) {
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export const toasts = new ToastStore();
