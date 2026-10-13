// Gera o instalador com o frontend compilado FORA da pasta do projeto (pasta temporária do SO).
// Em projetos dentro do Dropbox/OneDrive, dist/ fica travado pelo sincronizador (EPERM/EBUSY).
// Uso: npm run bundle [-- <args extras do `tauri build`>]
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const work = mkdtempSync(join(tmpdir(), "hsync-bundle-"));
const outDir = join(work, "dist");
const configFile = join(work, "tauri.bundle.json");
writeFileSync(
  configFile,
  JSON.stringify({
    build: {
      frontendDist: outDir,
      // O caminho vai por variável de ambiente: o Tauri não interpreta aspas no comando.
      beforeBuildCommand: "npx vite build --emptyOutDir",
    },
  }),
);

const result = spawnSync("npx", ["tauri", "build", "--config", configFile, ...process.argv.slice(2)], {
  stdio: "inherit",
  env: { ...process.env, HSYNC_OUT_DIR: outDir },
  shell: process.platform === "win32",
});
process.exit(result.status ?? 1);
