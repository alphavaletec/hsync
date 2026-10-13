//! Testes contra o binário real do rclone (sem nuvem: só diretórios temporários).
//! São pulados se `src-tauri/binaries/rclone-<triple>` não existir (rode scripts/fetch-rclone).
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use super::daemon::{parse_log_lines, parse_port};
use super::filters::Filters;
use super::RcClient;
use crate::secret::{random_token, Secret};

fn rclone_bin() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rclone-"))
    })
}

async fn start_rcd(bin: &Path, config: &Path) -> (Child, RcClient) {
    let user = Secret::from(random_token(8));
    let pass = Secret::from(random_token(16));
    let mut child = Command::new(bin)
        .args(["rcd", "--rc-addr", "127.0.0.1:0", "--ask-password=false", "--use-json-log"])
        .arg("--config")
        .arg(config)
        .env("RCLONE_RC_USER", user.expose())
        .env("RCLONE_RC_PASS", pass.expose())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn rclone");
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let port = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(p) = parse_log_lines(&line).iter().find_map(|e| parse_port(&e.msg)) {
                return p;
            }
        }
        panic!("rclone saiu sem informar a porta");
    })
    .await
    .expect("timeout esperando a porta");
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    let client = RcClient::new(port, user, pass).unwrap();
    client.noop().await.expect("noop");
    (child, client)
}

async fn wait_job(client: &RcClient, jobid: i64) -> super::types::JobStatus {
    for _ in 0..200 {
        let st = client.job_status(jobid).await.unwrap();
        if st.finished {
            return st;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("job {jobid} não terminou");
}

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_copy_bisync_against_real_rclone() {
    let Some(bin) = rclone_bin() else {
        eprintln!("binário do rclone ausente: teste pulado");
        return;
    };
    // Caminhos curtos: o bisync estoura MAX_PATH no Windows com caminhos longos.
    let tmp = tempfile::Builder::new().prefix("hs").tempdir().unwrap();
    let root = tmp.path();
    let (src, dst, bsrc, bdst, work) =
        (root.join("s"), root.join("d"), root.join("b1"), root.join("b2"), root.join("w"));
    write(&src.join("a.txt"), "a");
    write(&src.join("sub/b.txt"), "bb");
    write(&src.join("lixo.tmp"), "x");
    write(&src.join(".oculto"), "x");
    write(&src.join("node_modules/m.js"), "x");
    write(&dst.join("velho.txt"), "x");

    let (mut child, client) = start_rcd(&bin, &root.join("rclone.conf")).await;

    // --- auth obrigatória ---
    let unauth = reqwest::Client::builder().no_proxy().build().unwrap();
    let code = unauth.post(format!("{}/rc/noop", client.base_url())).send().await.unwrap().status();
    assert_eq!(code.as_u16(), 401);

    let filters = Filters {
        exclude_dirs: vec!["node_modules".into()],
        exclude_exts: vec!["tmp".into()],
        exclude_hidden: true,
        custom_rules: vec![],
    };
    let s = |p: &Path| p.to_string_lossy().to_string();

    // --- dry-run de sync: nada muda, mas as contagens aparecem ---
    let jobid = client
        .call_async(
            "sync/sync",
            json!({ "srcFs": s(&src), "dstFs": s(&dst), "_group": "t1",
                    "_filter": { "FilterRule": filters.to_rules() }, "_config": { "DryRun": true } }),
        )
        .await
        .unwrap();
    assert!(wait_job(&client, jobid).await.success);
    let stats = client.core_stats("t1").await.unwrap();
    assert_eq!(stats.transfers, 2, "dry-run deve contar a.txt e sub/b.txt");
    assert_eq!(stats.deletes, 1, "dry-run deve contar a exclusão de velho.txt");
    assert!(dst.join("velho.txt").exists());
    assert!(!dst.join("a.txt").exists());

    // --- sync real com filtros ---
    let jobid = client
        .call_async(
            "sync/sync",
            json!({ "srcFs": s(&src), "dstFs": s(&dst), "_group": "t2",
                    "_filter": { "FilterRule": filters.to_rules() } }),
        )
        .await
        .unwrap();
    assert!(wait_job(&client, jobid).await.success);
    assert!(dst.join("a.txt").exists());
    assert!(dst.join("sub/b.txt").exists());
    assert!(!dst.join("velho.txt").exists(), "sync remove o que não existe na origem");
    assert!(!dst.join("lixo.tmp").exists());
    assert!(!dst.join(".oculto").exists());
    assert!(!dst.join("node_modules").exists());
    let stats = client.core_stats("t2").await.unwrap();
    assert_eq!(stats.bytes, 3);
    client.stats_delete("t2").await.unwrap();

    // --- copy nunca remove ---
    write(&dst.join("extra.txt"), "x");
    let jobid = client
        .call_async("sync/copy", json!({ "srcFs": s(&src), "dstFs": s(&dst), "_group": "t3" }))
        .await
        .unwrap();
    assert!(wait_job(&client, jobid).await.success);
    assert!(dst.join("extra.txt").exists());

    // --- bisync: resync inicial e depois propagação nos dois sentidos ---
    write(&bsrc.join("um.txt"), "1");
    std::fs::create_dir_all(&bdst).unwrap();
    // Os mesmos parâmetros que o app usa (evita o teste divergir do código real).
    use crate::jobs::manager::bisync_params;
    let bis = |resync: bool| {
        let mut p = bisync_params(&s(&bsrc), &s(&bdst), &work, resync, false, false);
        p["_group"] = json!("t4");
        p
    };
    assert!(wait_job(&client, client.call_async("sync/bisync", bis(true)).await.unwrap()).await.success);
    assert!(bdst.join("um.txt").exists());
    write(&bdst.join("dois.txt"), "2");
    assert!(wait_job(&client, client.call_async("sync/bisync", bis(false)).await.unwrap()).await.success);
    assert!(bsrc.join("dois.txt").exists(), "bisync deve trazer arquivos do path2");

    // --- seleção por item (aba Arquivos): raiz inteira + regras compiladas ---
    use crate::jobs::selection::{compile_rules, ItemState, Selection};
    let cloud = root.join("c");
    let local = root.join("l");
    let work2 = root.join("w2");
    write(&cloud.join("Fotos/a.jpg"), "a");
    write(&cloud.join("Fotos/RAW/big.raw"), "r");
    write(&cloud.join("Docs/x.pdf"), "x");
    write(&cloud.join("Outros/nao.txt"), "n");
    let sels = vec![
        Selection { path: "Fotos".into(), is_dir: true, state: ItemState::Sync },
        Selection { path: "Fotos/RAW".into(), is_dir: true, state: ItemState::Ignore },
        Selection { path: "Docs/x.pdf".into(), is_dir: false, state: ItemState::Download },
    ];
    let sync_rules = compile_rules(&sels, ItemState::Sync).unwrap();
    let bis_sel = |resync: bool| {
        let mut p = bisync_params(&s(&local), &s(&cloud), &work2, resync, false, false);
        p["_group"] = json!("t5");
        p["_filter"] = json!({ "FilterRule": sync_rules });
        p
    };
    std::fs::create_dir_all(&local).unwrap();
    let st = wait_job(&client, client.call_async("sync/bisync", bis_sel(true)).await.unwrap()).await;
    assert!(st.success, "resync seletivo: {}", st.error);
    assert!(local.join("Fotos/a.jpg").exists());
    assert!(!local.join("Fotos/RAW").exists(), "pasta ignorada não desce");
    assert!(!local.join("Outros").exists(), "fora da seleção não desce");
    write(&local.join("Fotos/nova.jpg"), "n");
    let st = wait_job(&client, client.call_async("sync/bisync", bis_sel(false)).await.unwrap()).await;
    assert!(st.success, "bisync seletivo: {}", st.error);
    assert!(cloud.join("Fotos/nova.jpg").exists(), "mudança local sobe");
    assert!(cloud.join("Fotos/RAW/big.raw").exists(), "ignorado não é apagado na nuvem");

    // Renomear localmente vira renomear na nuvem (sobe o novo, remove o antigo), sem duplicar.
    std::fs::rename(local.join("Fotos/a.jpg"), local.join("Fotos/a-renomeado.jpg")).unwrap();
    let st = wait_job(&client, client.call_async("sync/bisync", bis_sel(false)).await.unwrap()).await;
    assert!(st.success, "bisync após renomear: {}", st.error);
    assert!(cloud.join("Fotos/a-renomeado.jpg").exists(), "nome novo sobe");
    assert!(!cloud.join("Fotos/a.jpg").exists(), "nome antigo sai da nuvem");
    assert!(!local.join("Fotos/a.jpg").exists(), "nome antigo não volta para o PC");

    let down_rules = compile_rules(&sels, ItemState::Download).unwrap();
    let jobid = client
        .call_async(
            "sync/sync",
            json!({ "srcFs": s(&cloud), "dstFs": s(&local), "_group": "t6", "_filter": { "FilterRule": down_rules } }),
        )
        .await
        .unwrap();
    let st = wait_job(&client, jobid).await;
    assert!(st.success, "download seletivo: {}", st.error);
    assert!(local.join("Docs/x.pdf").exists(), "arquivo marcado como offline desce");
    assert!(local.join("Fotos/a-renomeado.jpg").exists(), "espelho filtrado não apaga o que está fora do filtro");

    // --- "liberar espaço": a cópia local só sai se tudo nela existir igual na nuvem ---
    use crate::commands::files::{local_copy_in_cloud, LocalVsCloud};
    let (lc, cc) = (root.join("lc"), root.join("cc"));
    write(&lc.join("Pasta/a.txt"), "a");
    write(&cc.join("Pasta/a.txt"), "a");
    write(&cc.join("Pasta/so-nuvem.txt"), "n");
    write(&lc.join("doc.pdf"), "pdf");
    write(&cc.join("doc.pdf"), "pdf");
    write(&lc.join("mudou.txt"), "novo conteúdo");
    write(&cc.join("mudou.txt"), "antigo");
    let same_dir = local_copy_in_cloud(&client, &lc.join("Pasta"), &s(&cc.join("Pasta")), true).await.unwrap();
    assert_eq!(same_dir, LocalVsCloud::Same, "pasta igual (a nuvem pode ter a mais)");
    write(&lc.join("Pasta/so-local.txt"), "x");
    let only_local = local_copy_in_cloud(&client, &lc.join("Pasta"), &s(&cc.join("Pasta")), true).await.unwrap();
    assert_eq!(only_local, LocalVsCloud::Different { only_local: 1, differ: 0 });
    let same_file = local_copy_in_cloud(&client, &lc.join("doc.pdf"), &s(&cc), false).await.unwrap();
    assert_eq!(same_file, LocalVsCloud::Same, "arquivo igual; os vizinhos diferentes não contam");
    let changed = local_copy_in_cloud(&client, &lc.join("mudou.txt"), &s(&cc), false).await.unwrap();
    assert_eq!(changed, LocalVsCloud::Different { only_local: 0, differ: 1 });

    // --- erro limpo para remote inexistente ---
    let err = client.fsinfo("naoexiste:").await.unwrap_err().to_string();
    assert!(!err.contains('\u{1b}'));

    client.quit().await.unwrap();
    let exited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    assert!(exited.is_ok(), "core/quit deve encerrar o processo");
}
