# HSync

Gerenciador visual de sincronização e backup em nuvem (Google Drive, OneDrive, Mega e os demais provedores do rclone), construído com **Tauri v2 + Svelte 5** e usando o **rclone** como motor (sidecar).

- 100% local: o tráfego vai direto do seu computador para o provedor. Tokens e senhas ficam no `rclone.conf` deste PC, sem nenhum servidor intermediário.
- Modos **cópia**, **espelho** (sync) e **bidirecional** (bisync), com filtros, simulação (dry-run) e agendamento.
- **Aba Arquivos:** navega qualquer conta, mesclando a nuvem com a cópia local. Cada arquivo ou pasta recebe um estado: **⇄ sincronizar**, **↓ manter offline**, **↑ só enviar** ou **⊘ ignorar**. Também dá para baixar e enviar na hora.
- **Pré-visualização** sem sair do app: PDF, imagens, áudio/vídeo, texto/código e Office (Word, Excel e PowerPoint). Tudo é renderizado localmente, sem visualizadores online.
- **Painel com mapa de sincronização:** pastas locais → rclone → contas, com animação do sentido dos dados nas tarefas ativas.
- Monitoramento em tempo real: velocidade, progresso, arquivos em transferência, histórico e log.
- **Reautorizar** contas com login pelo navegador (token expirado ou acesso revogado), sem perder a configuração.
- Fica na bandeja: fechar a janela não interrompe os agendamentos.

## Arquitetura

```
Svelte (webview) ──invoke/eventos──▶ Rust (Tauri) ──HTTP 127.0.0.1:porta-aleatória──▶ rclone rcd
                                       │                (basic auth com credenciais efêmeras)
                                       └── SQLite: tarefas + histórico
```

- O webview **não tem permissão de rede nem de processos**. Só o Rust fala com o rclone.
- `rclone rcd --rc-addr 127.0.0.1:0` sobe com usuário e senha aleatórios (CSPRNG) passados por variável de ambiente, e não pela linha de comando. A porta é lida do log JSON.
- As variáveis `RCLONE_*` do ambiente do usuário são removidas antes do spawn, para que não alterem o daemon.
- No Windows, o rclone roda num *Job Object* com `KILL_ON_JOB_CLOSE`: se o HSync travar, o rclone morre junto.
- Watchdog: health check a cada 5 s e reinício com backoff exponencial. Depois de 5 falhas em 2 minutos, o app pede uma ação do usuário. Tarefas em andamento numa queda são marcadas como `interrompida`.
- Telemetria: polling de `core/stats` por grupo (`job-<id>`) a cada 1 s, com eventos `job-progress` para a UI.

### Seleção por item (aba Arquivos)

Cada conta tem uma pasta local base (padrão `~/HSync/<conta>`). Os estados marcados viram até três tarefas *gerenciadas*, todas com raiz `conta:` ↔ pasta base:

| Estado | Tarefa | Operação rclone |
|---|---|---|
| ⇄ sincronizar | `<conta> ⇄ sincronizados` | `bisync` (local = path1; resync com `resyncMode newer`) |
| ↓ manter offline | `<conta> ↓ offline` | `sync` nuvem → local (a pasta local espelha a nuvem) |
| ↑ só enviar | `<conta> ↑ envio` | `copy` local → nuvem (nunca apaga na nuvem) |
| ⊘ ignorar | — | exclui o item das tarefas dos ancestrais |

As regras são compiladas do caminho mais profundo para o mais raso (`+ /Fotos/**`, `- /Fotos/RAW/**`, …, `- **`). Assim, o estado mais específico de cada item prevalece. Ao alterar a seleção, o bisync refaz o resync, que une os dois lados e nunca apaga. Excluir uma tarefa gerenciada desmarca os itens daquele estado.

| Pasta | Conteúdo |
|---|---|
| `src-tauri/src/rclone/` | daemon (ciclo de vida), cliente RC, filtros, tipos |
| `src-tauri/src/jobs/` | modelo, SQLite, execução/monitor/agendador |
| `src-tauri/src/commands/` | comandos Tauri expostos ao frontend |
| `src/lib/api/` | wrappers tipados de `invoke` e eventos |
| `src/lib/components/` | views, dashboard, wizard de contas, formulário de tarefas |

## Desenvolvimento

Requisitos: Rust estável, Node 20+, [pré-requisitos do Tauri](https://v2.tauri.app/start/prerequisites/) (no Windows: WebView2 + MSVC Build Tools).

```bash
npm install
npm run fetch-rclone        # Windows; em Linux/macOS: scripts/fetch-rclone.sh
npm run tauri dev
```

O script baixa o **rclone v1.75.2** oficial, valida o SHA256 (hashes fixados no script) e grava em `src-tauri/binaries/rclone-<target-triple>[.exe]`. O binário não é versionado.

> **Projeto dentro do Dropbox/OneDrive?** Sincronizadores e antivírus travam arquivos de build (`os error 32` / `EBUSY`). O cache do Vite já fica na pasta temporária do sistema. Para o Rust, crie `src-tauri/.cargo/config.toml` (está no `.gitignore`):
> ```toml
> [build]
> target-dir = "C:/Users/<você>/AppData/Local/hsync-build/target"
> ```

### Testes

```bash
cd src-tauri && cargo test     # unitários + integração contra o rclone real (diretórios temporários)
npm run check                  # svelte-check
```

## Empacotamento (Windows)

```bash
npm run fetch-rclone
npm run tauri build            # instalador NSIS em <target-dir>/release/bundle/nsis/
```

`tauri.conf.json` declara `bundle.externalBin: ["binaries/rclone"]`. O Tauri procura `rclone-x86_64-pc-windows-msvc.exe` e o instala ao lado do `HSync.exe` como `rclone.exe`. Para ARM64, use `scripts/fetch-rclone.ps1 -Target aarch64-pc-windows-msvc` e `npm run tauri build -- --target aarch64-pc-windows-msvc`.

Para distribuição, assine o `HSync.exe` e o instalador. O SmartScreen e alguns antivírus sinalizam binários não assinados. No macOS, o sidecar precisa ser assinado e notarizado junto com o app.

## Segurança: notas e limitações

- **Senha mestre:** usa a criptografia nativa do rclone (NaCl secretbox, XSalsa20-Poly1305), e não AES-256. Assim o `rclone.conf` continua compatível com o rclone de linha de comando. A chave é derivada da senha por SHA-256, sem KDF lenta, então use uma senha longa. Durante a sessão, a senha fica em memória (zerada ao liberar) para que o daemon seja desbloqueado de novo após um reinício.
- **Google Drive:** o client_id compartilhado do rclone está sendo descontinuado em 2026. O wizard recomenda criar um próprio ([guia](https://rclone.org/drive/#making-your-own-client-id)).
- **OAuth:** é feito pelo próprio rclone (servidor local em `127.0.0.1:53682`). O token vai direto para o `rclone.conf`.
- **Bisync no Windows:** o rclone grava listagens cujo nome contém os dois caminhos. Caminhos muito longos estouram o limite de 260 caracteres, e o formulário valida isso. A primeira execução faz `--resync`. As seguintes usam `--recover --resilient`, e o limite de 50% de exclusões do rclone continua ativo.
- **Arquivos ocultos:** o filtro "ignorar ocultos" cobre dotfiles e arquivos de sistema do Windows (`desktop.ini`, `Thumbs.db`, lixeira). O atributo *Hidden* do NTFS não é suportado pelo rclone.
- **Limite de banda:** é global (`core/bwlimit`). O rclone não aplica limite por tarefa via RC.
- **Agendamento:** só roda com o HSync aberto (na bandeja). A opção "iniciar com o sistema" abre o app minimizado.
- **Pré-visualização:** o arquivo vai para um cache privado (`<app_local_data>/preview`), que é limpo a cada sessão. É o único diretório exposto ao webview pelo asset protocol. Links dentro de documentos não navegam, e "abrir no aplicativo" é validado no Rust: só arquivos do cache e só tipos de documento ou mídia (nunca executáveis). Formatos antigos (.doc, .ppt) não têm renderizador local.
