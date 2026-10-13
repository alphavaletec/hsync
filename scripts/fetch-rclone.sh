#!/usr/bin/env bash
# Baixa o binário oficial do Rclone (versão pinada), valida SHA256 e o coloca em
# src-tauri/binaries/rclone-<target-triple> para ser empacotado como sidecar do Tauri.
# Uso: scripts/fetch-rclone.sh [target-triple]   (padrão: triple do rustc do host)
set -euo pipefail

VERSION="v1.75.2"
TARGET="${1:-$(rustc -vV | sed -n 's/^host: //p')}"

# Hashes de https://downloads.rclone.org/v1.75.2/SHA256SUMS. Mantenha em sincronia com fetch-rclone.ps1.
case "$TARGET" in
  x86_64-unknown-linux-gnu)  ZIP="rclone-$VERSION-linux-amd64.zip";   SHA="349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702"; EXT="" ;;
  aarch64-unknown-linux-gnu) ZIP="rclone-$VERSION-linux-arm64.zip";   SHA="7e1e8d69654941b7b7df84ee74c5f7fb09cce7d5496947fb7bf16b55ff427d10"; EXT="" ;;
  x86_64-apple-darwin)       ZIP="rclone-$VERSION-osx-amd64.zip";     SHA="574e1f31cf1c464e5fb0325cb66fcbdeaf1efa1802bd9b17f13c256fd74b6561"; EXT="" ;;
  aarch64-apple-darwin)      ZIP="rclone-$VERSION-osx-arm64.zip";     SHA="610ab7d48615135dfc60b763858b11a426a0d3d1160c1bb76fb0ae6011b02edd"; EXT="" ;;
  x86_64-pc-windows-msvc)    ZIP="rclone-$VERSION-windows-amd64.zip"; SHA="8a9db2bad1dc79a65b3348fd857069598e2aab7cf4d118b77b1df23055aff818"; EXT=".exe" ;;
  aarch64-pc-windows-msvc)   ZIP="rclone-$VERSION-windows-arm64.zip"; SHA="7eb9844c9e5b723b9ef28821891caa17f6d9d1a375c49365cbe04114cec8dcd9"; EXT=".exe" ;;
  *) echo "Target não suportado: $TARGET" >&2; exit 1 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$ROOT/src-tauri/binaries"
DEST="$BIN_DIR/rclone-$TARGET$EXT"

if [[ -x "$DEST" ]] && [[ "$("$DEST" version 2>/dev/null | head -1)" == "rclone $VERSION" ]]; then
  echo "rclone $VERSION já presente em $DEST"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

URL="https://downloads.rclone.org/$VERSION/$ZIP"
echo "Baixando $URL"
curl -fsSL "$URL" -o "$WORK/$ZIP"

if command -v sha256sum >/dev/null; then GOT="$(sha256sum "$WORK/$ZIP" | cut -d' ' -f1)"; else GOT="$(shasum -a 256 "$WORK/$ZIP" | cut -d' ' -f1)"; fi
if [[ "$GOT" != "$SHA" ]]; then
  echo "SHA256 inválido para $ZIP: esperado $SHA, obtido $GOT" >&2
  exit 1
fi

unzip -q "$WORK/$ZIP" -d "$WORK/x"
mkdir -p "$BIN_DIR"
cp "$(find "$WORK/x" -type f -name "rclone$EXT" | head -1)" "$DEST"
chmod +x "$DEST"
echo "OK: $DEST"
