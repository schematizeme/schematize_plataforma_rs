//! Testes do fluxo de instalação do parasita (downloader injetado, binários falsos).
//!
//! **Onde:** `cargo test`; HOME sempre numa pasta temporária, nunca a real; zero rede.

use super::testes_decisao::{sem_env, tmp};
use super::*;
use std::cell::{Cell, RefCell};
use std::fs;
#[cfg(unix)]
use std::time::Instant;

const CFG: Config<'static> = Config::new("schematize");
const LINUX: (&str, &str) = ("linux", "x86_64");

/// **O quê:** downloader que grava um script que sai 0 (binário "válido").
///
/// **Onde:** os testes de sucesso.
fn baixa_ok(_: &str, dst: &Path) -> Result<(), String> {
    fs::write(dst, "#!/bin/sh\nexit 0\n").map_err(|e| e.to_string())
}

#[cfg(unix)]
/// **O quê:** downloader que grava um script com o corpo `corpo` (sh).
///
/// **Onde:** testes com binário falso (dorme, falha, etc.).
fn baixa_script(corpo: &'static str) -> impl Fn(&str, &Path) -> Result<(), String> {
    move |_, dst| fs::write(dst, format!("#!/bin/sh\n{corpo}\n")).map_err(|e| e.to_string())
}

#[cfg(unix)]
/// **O quê:** nomes de arquivo `*.novo*` sobrando em `pasta`.
///
/// **Onde:** os testes de limpeza.
fn sobras_novo(pasta: &Path) -> Vec<String> {
    match fs::read_dir(pasta) {
        Ok(rd) => rd
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.contains(".novo"))
            .collect(),
        Err(_) => vec![],
    }
}

#[cfg(unix)]
#[test]
fn instala_e_nao_deixa_novo() {
    let d = tmp("ok");
    let (lock, pasta) = (d.join("run/lock"), d.join("bin"));
    let r = garantir_em(&CFG, &pasta, &lock, LINUX, &baixa_ok);
    assert_eq!(r.instalados, NOMES.to_vec());
    assert!(r.falhas.is_empty() && r.avisos.is_empty(), "{r:?}");
    assert!(o_que_falta(&pasta).is_empty());
    assert!(sobras_novo(&pasta).is_empty());
    assert!(!lock.exists(), "lock tem de ser removido");
    assert!(linhas(&CFG, &r, &pasta)[0].contains("instalei o gestor"));
}

#[cfg(unix)]
#[test]
fn binario_que_nao_executa_remove_o_novo_e_poupa_o_destino() {
    let d = tmp("ruim");
    let pasta = d.join("bin");
    fs::create_dir_all(&pasta).unwrap();
    // O primeiro já existe: tem de ficar intocado. Falta só o -gui.
    let existente = pasta.join(NOMES[0]);
    fs::write(&existente, "original").unwrap();
    let ruim =
        |_: &str, dst: &Path| fs::write(dst, "isto nao e um binario").map_err(|e| e.to_string());
    let r = garantir_em(&CFG, &pasta, &d.join("lock"), LINUX, &ruim);
    assert_eq!(r.falhas.len(), 1, "{r:?}");
    assert_eq!(r.falhas[0].0, NOMES[1]);
    assert!(sobras_novo(&pasta).is_empty());
    assert!(!pasta.join(NOMES[1]).exists());
    assert_eq!(fs::read_to_string(&existente).unwrap(), "original");
    assert!(linhas(&CFG, &r, &pasta)[0].contains("o app segue normal"));
}

#[cfg(unix)]
#[test]
fn versao_que_sai_diferente_de_zero_falha_com_o_codigo() {
    let d = tmp("exit3");
    let r = garantir_em(&CFG, &d.join("bin"), &d.join("lock"), LINUX, &baixa_script("exit 3"));
    assert_eq!(r.falhas.len(), 2, "{r:?}");
    assert!(r.falhas[0].1.contains("`--version` saiu com"), "{:?}", r.falhas);
    assert!(r.instalados.is_empty());
}

#[cfg(unix)]
#[test]
fn versao_que_trava_estoura_o_timeout_e_nao_deixa_novo() {
    let d = tmp("timeout");
    let pasta = d.join("bin");
    let cfg = Config::new("schematize").com_timeout(Duration::from_millis(300));
    let t0 = Instant::now();
    let r = garantir_em(&cfg, &pasta, &d.join("lock"), LINUX, &baixa_script("exec sleep 30"));
    assert!(t0.elapsed() < Duration::from_secs(20), "o teto não foi respeitado");
    assert_eq!(r.falhas.len(), 2, "continua após a 1ª falha: {r:?}");
    assert!(r.falhas[0].1.contains("não terminou em"), "{:?}", r.falhas);
    assert!(sobras_novo(&pasta).is_empty());
    assert!(o_que_falta(&pasta).len() == 2, "nada foi instalado");
    assert!(!d.join("lock").exists(), "lock solto mesmo com falha");
}

#[cfg(unix)]
#[test]
fn filho_roda_com_sem_market_para_nao_recursar() {
    let d = tmp("semmarket");
    let marca = d.join("marca.txt");
    let corpo: &'static str =
        Box::leak(format!("printf %s \"$SCHEMATIZE_SEM_MARKET\" > '{}'", marca.display()).into());
    let r = garantir_em(&CFG, &d.join("bin"), &d.join("lock"), LINUX, &baixa_script(corpo));
    assert!(r.falhas.is_empty(), "{r:?}");
    assert_eq!(fs::read_to_string(&marca).unwrap(), "1");
}

#[cfg(unix)]
#[test]
fn varias_falhas_sao_todas_listadas_e_a_boa_instala() {
    let d = tmp("mistas");
    let pasta = d.join("bin");
    // O 1º binário falha no download, o 2º instala: a falha não pode abortar o seguinte.
    let n = Cell::new(0);
    let mista = |u: &str, dst: &Path| {
        n.set(n.get() + 1);
        if u.contains("-gui-") {
            baixa_ok(u, dst)
        } else {
            Err("rede caiu".to_string())
        }
    };
    let r = garantir_em(&CFG, &pasta, &d.join("lock"), LINUX, &mista);
    assert_eq!(n.get(), 2, "tentou os dois");
    assert_eq!(r.instalados, vec![NOMES[1]]);
    assert_eq!(r.falhas, vec![(NOMES[0], "rede caiu".to_string())]);
    let l = linhas(&CFG, &r, &pasta);
    assert_eq!(l.len(), 2);
    assert!(l[0].contains("instalei") && l[1].contains("rede caiu"));

    // Ambos falham: duas linhas de falha, uma por binário.
    let d2 = tmp("mistas2");
    let ruim = |_: &str, _: &Path| Err("sem rota".to_string());
    let r2 = garantir_em(&CFG, &d2.join("bin"), &d2.join("lock"), LINUX, &ruim);
    assert_eq!(r2.falhas.len(), 2);
    assert_eq!(linhas(&CFG, &r2, &d2).len(), 2);
}

#[test]
fn lock_recente_nao_faz_nada() {
    let d = tmp("lock");
    let lock = d.join("lock");
    fs::write(&lock, "outro").unwrap();
    let chamou = Cell::new(false);
    let espia = |_: &str, _: &Path| {
        chamou.set(true);
        Err("não devia".to_string())
    };
    let r = garantir_em(&CFG, &d.join("bin"), &lock, LINUX, &espia);
    assert!(r.outro_cuida && r.instalados.is_empty() && r.falhas.is_empty());
    assert!(!chamou.get());
    assert!(lock.exists(), "o lock do outro processo não é nosso para remover");
    assert!(linhas(&CFG, &r, &d).is_empty());
}

#[test]
fn lock_orfao_e_removido_e_o_trabalho_segue() {
    let d = tmp("orfao");
    let lock = d.join("lock");
    fs::write(&lock, "morto").unwrap();
    let velho = std::time::SystemTime::now() - Duration::from_secs(11 * 60);
    fs::File::options().write(true).open(&lock).unwrap().set_modified(velho).unwrap();
    assert_eq!(tomar_lock(&lock), Ok(Lock::Meu));
    assert_eq!(fs::read_to_string(&lock).unwrap(), std::process::id().to_string());
}

#[test]
fn lock_com_data_no_futuro_conta_como_orfao() {
    let d = tmp("futuro");
    let lock = d.join("lock");
    fs::write(&lock, "torto").unwrap();
    let futuro = std::time::SystemTime::now() + Duration::from_secs(3600);
    fs::File::options().write(true).open(&lock).unwrap().set_modified(futuro).unwrap();
    assert_eq!(tomar_lock(&lock), Ok(Lock::Meu));
}

#[test]
fn lock_em_pasta_impossivel_vira_falha_e_nao_panico() {
    let d = tmp("lockruim");
    fs::write(d.join("arquivo"), "x").unwrap();
    // O "diretório" do lock é um arquivo: criar_dir_all falha.
    let r = garantir_em(&CFG, &d.join("bin"), &d.join("arquivo/lock"), LINUX, &baixa_ok);
    assert_eq!(r.falhas.len(), 1, "{r:?}");
    assert!(r.falhas[0].1.contains("criar"), "{:?}", r.falhas);
}

#[test]
fn sem_asset_nao_baixa_e_diz_o_motivo() {
    let d = tmp("semasset");
    let chamou = Cell::new(false);
    let espia = |_: &str, _: &Path| {
        chamou.set(true);
        Ok(())
    };
    let r = garantir_em(&CFG, &d.join("bin"), &d.join("lock"), ("linux", "aarch64"), &espia);
    assert_eq!(r.falhas.len(), 2, "uma por binário: {r:?}");
    assert!(r.falhas[0].1.contains("sem binário publicado"), "{:?}", r.falhas);
    assert!(!chamou.get());
    assert!(!d.join("lock").exists());
}

#[test]
fn urls_pedidas_ao_downloader_sao_as_da_plataforma() {
    let d = tmp("urls");
    let vistas = RefCell::new(Vec::new());
    let espia = |u: &str, _: &Path| {
        vistas.borrow_mut().push(u.to_string());
        Err("parar".to_string())
    };
    for (os, arch) in [("freebsd", "x86_64"), ("macos", "aarch64"), ("windows", "x86_64")] {
        vistas.borrow_mut().clear();
        let sub = d.join(os);
        let _ = garantir_em(&CFG, &sub.join("bin"), &sub.join("lock"), (os, arch), &espia);
        let v = vistas.borrow();
        assert_eq!(v.len(), 2, "{os}");
        let arch_asset = if arch == "aarch64" { "arm64" } else { arch };
        let ext = if os == "windows" { ".exe" } else { "" };
        assert!(
            v[0].ends_with(&format!("schematize-market-{os}-{arch_asset}{ext}")),
            "{os}: {}",
            v[0]
        );
        assert!(v[1].contains("schematize-market-gui"));
    }
}

#[cfg(unix)]
#[test]
fn windows_grava_exe_e_novo_exe() {
    let d = tmp("winflow");
    let pasta = d.join("bin");
    let vistos = RefCell::new(Vec::new());
    let espia = |u: &str, dst: &Path| {
        vistos.borrow_mut().push(dst.file_name().unwrap().to_string_lossy().into_owned());
        baixa_ok(u, dst)
    };
    let r = garantir_em(&CFG, &pasta, &d.join("lock"), ("windows", "x86_64"), &espia);
    assert!(r.falhas.is_empty(), "{r:?}");
    assert_eq!(*vistos.borrow(), ["schematize-market.novo.exe", "schematize-market-gui.novo.exe"]);
    assert!(pasta.join("schematize-market.exe").is_file());
    assert!(pasta.join("schematize-market-gui.exe").is_file());
    assert!(sobras_novo(&pasta).is_empty());
}

#[test]
fn caminho_rapido_nao_baixa_nem_trava() {
    let d = tmp("rapido");
    let pasta = d.join("bin");
    fs::create_dir_all(&pasta).unwrap();
    for n in NOMES {
        fs::write(pasta.join(nome_do_arquivo(n, std::env::consts::OS)), "x").unwrap();
    }
    let espia = |_: &str, _: &Path| Err("não devia".to_string());
    let (os, arch) = plataforma_atual();
    let r = garantir_em(&CFG, &pasta, &d.join("lock"), (os, arch), &espia);
    assert!(r.ja_estava);
    assert!(!d.join("lock").exists());
}

#[test]
fn downloader_com_curl_inexistente_da_erro_legivel() {
    let d = tmp("curl");
    let e = baixar_com_programa("curl-que-nao-existe-42", "http://invalido.test/x", &d.join("o"))
        .unwrap_err();
    assert!(e.contains("curl-que-nao-existe-42 indisponível"), "{e}");
}

#[test]
fn opt_out_nao_chama_o_downloader() {
    let d = tmp("optout");
    let chamou = Cell::new(false);
    let espia = |_: &str, _: &Path| {
        chamou.set(true);
        Ok(())
    };
    for var in ["SCHEMATIZE_SEM_MARKET", "SCHEMATIZE_DELEGADO_AO_MARKET"] {
        let env = move |k: &str| (k == var).then(|| "1".to_string());
        assert!(garantir_com(&CFG, &env, &[], Some(d.clone()), LINUX, &espia).is_none(), "{var}");
    }
    let ci = |k: &str| (k == "CI").then(|| "true".to_string());
    assert!(garantir_com(&CFG, &ci, &[], Some(d.clone()), LINUX, &espia).is_none());
    assert!(!chamou.get());
    assert!(!d.join(".cargo").exists());
}

#[test]
fn sem_home_vira_falha_relatada() {
    let r = garantir_com(&CFG, &sem_env, &[], None, LINUX, &baixa_ok).unwrap();
    assert_eq!(r.falhas.len(), 1);
    assert!(r.falhas[0].1.contains("HOME"), "{:?}", r.falhas);
}

#[cfg(unix)]
#[test]
fn fluxo_completo_por_garantir_com_e_home_hostil() {
    // HOMEs hostis: com espaços e unicode funcionam; o real nunca é tocado.
    for nome in ["home com espaço", "josé-日本語", "a b/c d"] {
        let d = tmp("fluxo").join(nome);
        fs::create_dir_all(&d).unwrap();
        let r = garantir_com(&CFG, &sem_env, &[], Some(d.clone()), LINUX, &baixa_ok).unwrap();
        assert_eq!(r.instalados.len(), 2, "{nome}: {r:?}");
        assert!(o_que_falta(&d.join(".cargo/bin")).is_empty(), "{nome}");
        // Segunda vez: caminho rápido.
        let r2 = garantir_com(&CFG, &sem_env, &[], Some(d), LINUX, &baixa_ok).unwrap();
        assert!(r2.ja_estava, "{nome}");
    }
}

#[test]
fn garantir_e_inerte_sob_cfg_test() {
    // Se não fosse inerte, tentaria baixar para o HOME real deste teste.
    garantir(&CFG);
}
