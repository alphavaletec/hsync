// Esvazia dist/ antes do build (o Vite não esvazia: emptyOutDir=false).
// Sincronizadores (Dropbox/OneDrive) e antivírus seguram arquivos por instantes: apagar e recriar
// a própria pasta falha com EPERM enquanto ela está "pendente de exclusão" no Windows. Por isso a
// pasta é mantida, só o conteúdo sai (com novas tentativas), e ela é marcada como ignorada no Dropbox.
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const dist = fileURLToPath(new URL("../dist", import.meta.url));

if (existsSync(dist)) {
  for (const entry of readdirSync(dist)) {
    rmSync(join(dist, entry), { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  }
} else {
  mkdirSync(dist, { recursive: true });
}

if (process.platform === "win32") {
  try {
    // Fluxo de dados alternativo do NTFS que o Dropbox reconhece como "ignorar esta pasta".
    writeFileSync(`${dist}:com.dropbox.ignored`, "1");
  } catch {
    // Sem NTFS ou sem Dropbox: irrelevante.
  }
}
