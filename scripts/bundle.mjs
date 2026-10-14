// Gera o instalador com o frontend compilado FORA da pasta do projeto (pasta temporária do SO).
// Em projetos dentro do Dropbox/OneDrive, dist/ fica travado pelo sincronizador (EPERM/EBUSY).
// Uso: npm run bundle [-- <args extras do `tauri build`>]
//
// Atualização automática: o instalador é assinado com a chave em ~/.tauri/hsync.key (ou na variável
// TAURI_SIGNING_PRIVATE_KEY). Sem chave, o instalador sai normal, mas sem os arquivos de atualização.
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const work = mkdtempSync(join(tmpdir(), "hsync-bundle-"));
const outDir = join(work, "dist");
const configFile = join(work, "tauri.bundle.json");

const env = { ...process.env, HSYNC_OUT_DIR: outDir };
const defaultKey = join(homedir(), ".tauri", "hsync.key");
if (!env.TAURI_SIGNING_PRIVATE_KEY && existsSync(defaultKey)) env.TAURI_SIGNING_PRIVATE_KEY = defaultKey;
const signed = !!env.TAURI_SIGNING_PRIVATE_KEY;
if (!signed) console.warn("Aviso: sem chave de assinatura — o instalador não terá os arquivos de atualização automática.");

writeFileSync(
  configFile,
  JSON.stringify({
    build: {
      frontendDist: outDir,
      // O caminho vai por variável de ambiente: o Tauri não interpreta aspas no comando.
      beforeBuildCommand: "npx vite build --emptyOutDir",
    },
    bundle: { createUpdaterArtifacts: signed },
  }),
);

const result = spawnSync("npx", ["tauri", "build", "--config", configFile, ...process.argv.slice(2)], {
  stdio: "inherit",
  env,
  shell: process.platform === "win32",
});
process.exit(result.status ?? 1);
