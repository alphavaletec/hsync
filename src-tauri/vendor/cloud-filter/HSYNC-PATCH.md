# cloud-filter 0.0.6 — cópia com correções do HSync

Original: https://crates.io/crates/cloud-filter/0.0.6 (MIT, ver `LICENSE`). Mudanças:

- `src/filter/proxy.rs`: os callbacks não fazem mais `.unwrap()` ao avisar falha ao Windows.
  Um pânico dentro de `extern "system"` encerra o processo inteiro.
- `src/command/commands.rs` + `proxy.rs`: a falha de `fetch_data` é enviada sobre o trecho
  pedido pelo Windows (`RequiredFileOffset`/`RequiredLength`). Com `Length: 0` o Windows
  rejeita com `ERROR_CLOUD_FILE_INVALID_REQUEST` e o pedido fica pendurado.
- `tests/` e o alvo `[[test]]` foram removidos da cópia.
