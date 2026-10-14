// Atualiza a versão do app nos três lugares que precisam concordar.
// Uso: npm run version:set -- 0.1.1
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("Uso: npm run version:set -- <maior.menor.correção>   (ex.: 0.1.1)");
  process.exit(1);
}

const json = (path) => {
  const text = readFileSync(path, "utf8");
  writeFileSync(path, text.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`));
};
json("package.json");
json("src-tauri/tauri.conf.json");

const cargo = readFileSync("src-tauri/Cargo.toml", "utf8");
writeFileSync("src-tauri/Cargo.toml", cargo.replace(/^version = "[^"]+"/m, `version = "${version}"`));
console.log(`versão ${version} definida em package.json, tauri.conf.json e Cargo.toml`);
