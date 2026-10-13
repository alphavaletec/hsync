<script lang="ts">
  // Wizard de nova conta: provedor → dados → (perguntas / OAuth no navegador) → teste de conexão.
  import { Cloud, ExternalLink, LoaderCircle, TriangleAlert } from "@lucide/svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onDestroy } from "svelte";
  import { api, errorMessage } from "../../api/commands";
  import type { ConfigStep, Provider, RcloneOption, RemoteTest } from "../../api/types";
  import { app } from "../../stores/app.svelte";
  import { formatBytes, providerLabel } from "../../utils/format";
  import Modal from "../ui/Modal.svelte";
  import ConfigQuestion from "./ConfigQuestion.svelte";

  // Com `reauth`, o wizard só renova o login de uma conta existente (pula provedor/dados).
  let { onclose, reauth = null }: { onclose: () => void; reauth?: { name: string; kind: string } | null } = $props();

  type Phase = "provider" | "details" | "starting" | "question" | "oauth" | "testing" | "done" | "error";

  const FEATURED = [
    { kind: "drive", name: "gdrive", hint: "login pelo navegador" },
    { kind: "onedrive", name: "onedrive", hint: "login pelo navegador" },
    { kind: "mega", name: "mega", hint: "e-mail e senha" },
  ];
  const HELP_URL: Record<string, string> = {
    drive: "https://rclone.org/drive/#making-your-own-client-id",
    onedrive: "https://rclone.org/onedrive/#getting-your-own-client-id-and-key",
  };
  // Campos que o wizard já trata de forma dedicada para os provedores em destaque.
  const DEDICATED: Record<string, string[]> = {
    drive: ["client_id", "client_secret", "scope"],
    onedrive: ["client_id", "client_secret"],
    mega: ["user", "pass"],
  };

  // svelte-ignore state_referenced_locally
  let phase = $state<Phase>(reauth ? "starting" : "provider");
  let providers = $state<Provider[]>([]);
  // svelte-ignore state_referenced_locally
  let kind = $state(reauth?.kind ?? "");
  // svelte-ignore state_referenced_locally
  let name = $state(reauth?.name ?? "");
  let params = $state<Record<string, string>>({});
  let otherKind = $state("");
  let showKeys = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let question = $state<{ state: string; option: RcloneOption; error: string | null } | null>(null);
  let authUrl = $state<string | null>(null);
  let test = $state<RemoteTest | null>(null);
  let created = false;
  let pollTimer: ReturnType<typeof setTimeout> | undefined;

  const provider = $derived(providers.find((p) => p.name === kind));
  const genericOptions = $derived(
    (provider?.options ?? []).filter((o) => !(DEDICATED[kind] ?? []).includes(o.name)),
  );
  const requiredMissing = $derived(
    kind === "mega"
      ? !params.user || !params.pass
      : genericOptions.some((o) => o.required && !params[o.name] && !o.default),
  );

  $effect(() => {
    if (!reauth) return;
    api
      .reauthorizeRemote(reauth.name, reauth.kind)
      .then(handle)
      .catch((e) => fail(errorMessage(e)));
  });

  $effect(() => {
    if (reauth) return;
    api.listProviders().then((p) => (providers = p)).catch((e) => (error = errorMessage(e)));
  });

  onDestroy(() => clearTimeout(pollTimer));

  function choose(k: string, defaultName: string) {
    kind = k;
    let n = defaultName;
    for (let i = 2; app.remotes.some((r) => r.name.toLowerCase() === n.toLowerCase()); i++) n = `${defaultName}${i}`;
    name = n;
    params = k === "drive" ? { scope: "drive" } : {};
    phase = "details";
  }

  async function handle(step: ConfigStep) {
    const previousState = question?.state;
    question = null;
    if (step.step === "done") {
      created = true;
      phase = "testing";
      await app.refreshRemotes();
      try {
        test = await api.testRemote(name);
      } catch (e) {
        test = { ok: false, error: errorMessage(e), info: null, about: null };
      }
      phase = "done";
    } else if (step.step === "question") {
      if (!step.option) {
        // Passo sem pergunta (o provedor só devolveu um erro): não há como continuar daqui.
        fail(step.error ?? "O provedor não devolveu a próxima etapa da configuração.");
        return;
      }
      // Mesma pergunta de novo sem erro do provedor: deixa claro em vez de parecer travado.
      const repeated = previousState === step.state && !step.error;
      question = {
        state: step.state,
        option: step.option,
        error: repeated ? "O provedor pediu esta informação de novo. Confira a escolha e continue." : step.error,
      };
      phase = "question";
    } else {
      phase = "oauth";
      authUrl = null;
      poll(step.jobId);
    }
  }

  function poll(jobId: number) {
    pollTimer = setTimeout(async () => {
      try {
        const p = await api.oauthProgress(name, kind, jobId);
        if (!p.finished) {
          authUrl = p.authUrl;
          poll(jobId);
        } else if (p.error) {
          fail(p.error);
        } else if (p.next) {
          await handle(p.next);
        }
      } catch (e) {
        fail(errorMessage(e));
      }
    }, 1000);
  }

  function fail(msg: string) {
    error = msg;
    phase = "error";
  }

  async function submit() {
    busy = true;
    error = null;
    try {
      await handle(await api.createRemote(name, kind, $state.snapshot(params)));
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  async function answer(value: string) {
    if (!question || busy) return;
    busy = true;
    try {
      await handle(await api.answerConfig(name, kind, question.state, value));
    } catch (e) {
      fail(errorMessage(e));
    } finally {
      busy = false;
    }
  }

  async function cancel() {
    clearTimeout(pollTimer);
    if (!created && phase !== "provider" && phase !== "details") {
      await api.cancelRemoteConfig(name).catch(() => {});
    }
    onclose();
  }
</script>

<Modal title={reauth ? `reautorizar — ${reauth.name}:` : "conectar conta"} width="640px" onclose={cancel}>
  {#if phase === "provider"}
    <div class="sec-label mb-3">provedor</div>
    <div class="grid grid-cols-3 gap-4 px-2">
      {#each FEATURED as f (f.kind)}
        <button class="node px-4 py-4 text-left hover:border-accent" onclick={() => choose(f.kind, f.name)}>
          <span class="node-dot"></span>
          <Cloud size={18} class="text-accent" />
          <div class="mt-2 font-semibold">{providerLabel(f.kind)}</div>
          <div class="text-[11px] text-muted">{f.hint}</div>
        </button>
      {/each}
    </div>
    <div class="sec-label mt-6 mb-3">outro provedor</div>
    <div class="flex gap-2">
      <select class="input" bind:value={otherKind}>
        <option value="">— {providers.length ? `${providers.length} provedores do rclone` : "carregando…"} —</option>
        {#each providers as p (p.name)}
          <option value={p.name}>{p.description} ({p.name})</option>
        {/each}
      </select>
      <button class="btn" disabled={!otherKind} onclick={() => choose(otherKind, otherKind)}>continuar</button>
    </div>
    <p class="mt-6 text-[11.5px] text-dim">
      ◇ as credenciais ficam só neste computador, no rclone.conf. O tráfego vai direto daqui para o provedor.
    </p>
  {:else if phase === "details"}
    <div class="flex flex-col gap-4">
      <div>
        <label class="field-label" for="remote-name">nome da conta (usado nos caminhos, ex.: {name || "gdrive"}:Fotos)</label>
        <input id="remote-name" class="input" bind:value={name} maxlength="64" />
      </div>

      {#if kind === "drive" || kind === "onedrive"}
        <div class="term flex items-start gap-3 px-4 py-3 text-[12.5px]">
          <ExternalLink size={16} class="mt-0.5 shrink-0 text-accent" />
          <span>
            Ao clicar em <span class="text-accent">conectar</span>, o navegador abre na página de login
            {kind === "drive" ? "do Google" : "da Microsoft"}. Entre na sua conta e autorize — o HSync recebe o acesso
            direto neste PC. Não é preciso criar chaves.
          </span>
        </div>
        <button class="self-start text-[11.5px] text-muted hover:text-fg" onclick={() => (showKeys = !showKeys)}>
          {showKeys ? "▾" : "▸"} avançado: usar meu próprio client_id
        </button>
        {#if showKeys}
          {#if kind === "drive"}
            <div class="flex items-start gap-2 border border-warn/40 bg-warn/10 px-3 py-2 text-[12px] text-warn">
              <TriangleAlert size={15} class="mt-0.5 shrink-0" />
              <span>
                Sem client_id próprio, o login usa a chave compartilhada do rclone, que o Google está descontinuando em
                2026 e pode ficar lenta ou falhar. Com uma chave sua (gratuita) a conexão não depende disso.
              </span>
            </div>
          {/if}
          <div class="grid grid-cols-2 gap-3">
            <div>
              <label class="field-label" for="cid">client_id</label>
              <input id="cid" class="input" bind:value={params.client_id} autocomplete="off" />
            </div>
            <div>
              <label class="field-label" for="csec">client_secret</label>
              <input id="csec" class="input" type="password" bind:value={params.client_secret} autocomplete="off" />
            </div>
          </div>
          <button class="flex items-center gap-1 self-start text-[11.5px] text-accent hover:underline" onclick={() => openUrl(HELP_URL[kind])}>
            <ExternalLink size={12} /> como criar meu client_id
          </button>
        {/if}
        {#if kind === "drive"}
          <div>
            <label class="field-label" for="scope">acesso</label>
            <select id="scope" class="input" bind:value={params.scope}>
              <option value="drive">completo — todos os arquivos do Drive</option>
              <option value="drive.file">restrito — só arquivos criados pelo HSync</option>
              <option value="drive.readonly">somente leitura</option>
            </select>
          </div>
        {/if}
      {:else if kind === "mega"}
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="field-label" for="mu">e-mail</label>
            <input id="mu" class="input" bind:value={params.user} autocomplete="off" />
          </div>
          <div>
            <label class="field-label" for="mp">senha</label>
            <input id="mp" class="input" type="password" bind:value={params.pass} autocomplete="off" />
          </div>
        </div>
      {/if}

      {#if !DEDICATED[kind]}
        {#each genericOptions as o (o.name)}
          <div>
            <label class="field-label" for="opt-{o.name}">{o.name}{o.required ? " *" : ""}</label>
            {#if o.examples.length && o.kind !== "bool"}
              <select id="opt-{o.name}" class="input" bind:value={params[o.name]}>
                <option value="">{o.default ? `padrão (${o.default})` : "—"}</option>
                {#each o.examples as ex (ex.value)}
                  <option value={ex.value}>{ex.help || ex.value}</option>
                {/each}
              </select>
            {:else}
              <input
                id="opt-{o.name}"
                class="input"
                type={o.isPassword ? "password" : "text"}
                bind:value={params[o.name]}
                placeholder={o.default}
                autocomplete="off"
              />
            {/if}
            <p class="mt-1 line-clamp-2 text-[11px] text-dim" title={o.help}>{o.help}</p>
          </div>
        {/each}
      {/if}

      {#if error}<p class="selectable text-[12px] text-err">✕ {error}</p>{/if}
    </div>
  {:else if phase === "question" && question}
    <div class="sec-label mb-3">{providerLabel(kind)} · {question.option.Name}</div>
    {#key `${question.state}|${question.option.Name}`}
      <ConfigQuestion option={question.option} error={question.error} {busy} onanswer={answer} />
    {/key}
  {:else if phase === "oauth"}
    <div class="flex flex-col items-center gap-4 py-6 text-center">
      <LoaderCircle size={28} class="spin text-accent" />
      <h3 class="text-[16px] font-semibold">conclua a autorização no navegador</h3>
      <p class="max-w-[440px] text-[12px] text-muted">
        O rclone abriu a página de login de {providerLabel(kind)}. Depois de autorizar, volte aqui — esta tela avança
        sozinha. O retorno vai direto para este PC (127.0.0.1:53682).
      </p>
      {#if authUrl}
        <button class="btn" onclick={() => openUrl(authUrl!)}><ExternalLink size={14} /> abrir de novo no navegador</button>
      {/if}
    </div>
  {:else if phase === "starting"}
    <div class="flex items-center justify-center gap-3 py-10 text-muted">
      <LoaderCircle size={18} class="spin" /> preparando a renovação do login de {name}:…
    </div>
  {:else if phase === "testing"}
    <div class="flex items-center justify-center gap-3 py-10 text-muted">
      <LoaderCircle size={18} class="spin" /> testando conexão com {name}:…
    </div>
  {:else if phase === "done"}
    <div class="flex flex-col gap-3 py-2">
      {#if test?.ok}
        <h3 class="text-[16px] font-semibold"><span class="text-ok">◆</span> {name}: {reauth ? "login renovado" : "conectada"}</h3>
        {#if test.about?.total}
          <p class="text-[12px] text-muted">
            espaço: {formatBytes(test.about.used ?? 0)} usados de {formatBytes(test.about.total)}
            {#if test.about.free != null}· {formatBytes(test.about.free)} livres{/if}
          </p>
          <div class="progress"><div style:width="{((test.about.used ?? 0) / test.about.total) * 100}%"></div></div>
        {/if}
      {:else}
        <h3 class="text-[16px] font-semibold"><span class="text-warn">◇</span> conta salva, mas o teste falhou</h3>
        <p class="selectable text-[12px] text-err">{test?.error}</p>
      {/if}
    </div>
  {:else if phase === "error"}
    <div class="py-4">
      <h3 class="text-[16px] font-semibold"><span class="text-err">✕</span> {reauth ? "não foi possível reautorizar" : "não foi possível conectar"}</h3>
      {#if reauth}<p class="mt-2 text-[12px] text-muted">a conta continua configurada como estava.</p>{/if}
      <p class="selectable mt-2 text-[12px] text-err">{error}</p>
    </div>
  {/if}

  {#snippet footer()}
    {#if phase === "details"}
      <button class="btn mr-auto" onclick={() => ((phase = "provider"), (error = null))}>voltar</button>
      <button class="btn" onclick={cancel}>cancelar</button>
      <button class="btn btn-primary" onclick={submit} disabled={busy || !name.trim() || requiredMissing}>
        {busy ? "conectando…" : "conectar"}
      </button>
    {:else if phase === "done"}
      <button class="btn btn-primary" onclick={onclose}>pronto</button>
    {:else if phase === "error"}
      <button class="btn" onclick={onclose}>fechar</button>
      {#if !reauth}
        <button class="btn btn-primary" onclick={() => ((phase = "details"), (error = null))}>tentar de novo</button>
      {/if}
    {:else}
      <button class="btn" onclick={cancel}>cancelar</button>
    {/if}
  {/snippet}
</Modal>
