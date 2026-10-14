// Prepara os arquivos de uma versão para publicar no GitHub Releases:
//   HSync_<versão>_x64-setup.exe, a assinatura (.sig) e o latest.json que os apps instalados consultam.
// Uso: npm run release -- --notes "O que mudou"   (--skip-build reaproveita o último instalador compilado)
// Os arquivos saem em release/v<versão>/ (pasta ignorada pelo git).
// Antes: npm run version:set -- <nova versão>  (o número precisa ser maior que o da versão instalada).
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const REPO = "alphavaletec/hsync";
const args = process.argv.slice(2);
const notesAt = args.indexOf("--notes");
const notes = notesAt >= 0 ? args[notesAt + 1] : "";

const version = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).version;

if (!args.includes("--skip-build")) {
  const build = spawnSync("node", ["scripts/bundle.mjs"], { stdio: "inherit", shell: process.platform === "win32" });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

// Pasta de saída do cargo (pode estar fora do projeto: ver src-tauri/.cargo/config.toml).
// Obs.: se o Node roda de um app empacotado (MSIX), o Windows redireciona o AppData; por isso
// o resultado final é copiado para dentro do projeto.
let target = resolve("src-tauri/target");
const cfg = "src-tauri/.cargo/config.toml";
if (existsSync(cfg)) {
  const m = readFileSync(cfg, "utf8").match(/target-dir\s*=\s*"([^"]+)"/);
  if (m) target = resolve(m[1]);
}
const nsis = join(target, "release", "bundle", "nsis");
const exeName = `HSync_${version}_x64-setup.exe`;
const exe = join(nsis, exeName);
const sig = `${exe}.sig`;
if (!existsSync(exe) || !existsSync(sig)) {
  console.error(`Não encontrei ${exeName} e ${exeName}.sig em ${nsis}. A chave de assinatura estava disponível?`);
  process.exit(1);
}

const out = resolve("release", `v${version}`);
mkdirSync(out, { recursive: true });
copyFileSync(exe, join(out, exeName));
copyFileSync(sig, join(out, `${exeName}.sig`));
writeFileSync(
  join(out, "latest.json"),
  JSON.stringify(
    {
      version,
      notes,
      pub_date: new Date().toISOString(),
      platforms: {
        "windows-x86_64": {
          signature: readFileSync(sig, "utf8").trim(),
          url: `https://github.com/${REPO}/releases/download/v${version}/${exeName}`,
        },
      },
    },
    null,
    2,
  ),
);

console.log(`\nPronto: ${out}
Publique em https://github.com/${REPO}/releases/new :
  1. tag: v${version}
  2. anexe os 3 arquivos dessa pasta (instalador, .sig e latest.json)
  3. publique (não como rascunho nem pré-lançamento)
Os apps instalados passam a oferecer a atualização na próxima verificação.`);
