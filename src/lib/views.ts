import type { View } from "./stores/app.svelte";

/** Abas do app, na ordem dos atalhos Ctrl+1…5. */
export const VIEWS: { id: View; label: string; summary: string }[] = [
  { id: "dashboard", label: "painel", summary: "o que está conectado, o que está sincronizando agora e o histórico" },
  { id: "files", label: "arquivos", summary: "navegue pelas contas e escolha o que sincroniza, fica offline ou é ignorado" },
  { id: "jobs", label: "tarefas", summary: "sincronizações configuradas: origem, destino, modo e agendamento" },
  { id: "accounts", label: "contas", summary: "conecte e teste suas nuvens — Google Drive, OneDrive, Mega e outras" },
  { id: "settings", label: "config", summary: "senha mestre, limite de banda e inicialização com o sistema" },
];
