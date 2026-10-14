//! O parasita visto de FORA (API pública), como um host o usaria: roda nos comandos comuns,
//! NUNCA nos hooks. Porta os testes de integração do `schematize_cli_rs` sem precisar de binário.
//!
//! **Onde:** `cargo test --features parasita`. Sem rede: o programa de download é inexistente
//! (o mesmo efeito do `PATH=/nonexistent` do original) e o HOME é uma pasta de teste.
#![cfg(feature = "parasita")]

use schematize_plataforma::parasita::{
    baixar_com_programa, garantir_com, Config, Resultado, HOOKS_OVERDEV,
};
use std::path::{Path, PathBuf};

const CFG: Config<'static> = Config::new("schematize").com_hooks(HOOKS_OVERDEV);

/// **O quê:** HOME vazio e único por teste, dentro do `target`.
///
/// **Onde:** os testes daqui.
fn home(nome: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("parasita-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("criar o HOME de teste");
    d
}

/// **O quê:** roda `garantir_com` com ambiente limpo e um "curl" que não existe.
///
/// **Onde:** os testes daqui.
fn rodar(h: &Path, args: &[&str]) -> Option<Resultado> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let sem_curl = |u: &str, d: &Path| baixar_com_programa("curl-inexistente-42", u, d);
    garantir_com(&CFG, &|_| None, &args, Some(h.to_path_buf()), ("linux", "x86_64"), &sem_curl)
}

/// Controle: um comando comum TENTA instalar (e, sem curl, diz por quê). Sem isto, o teste
/// dos hooks passaria mesmo com o parasita quebrado.
#[test]
fn comando_comum_tenta_instalar_e_o_app_segue() {
    let h = home("comum");
    let r = rodar(&h, &["lang", "--list"]).expect("tem de rodar");
    assert_eq!(r.falhas.len(), 2, "uma por binário: {r:?}");
    assert!(r.falhas[0].1.contains("indisponível"), "{:?}", r.falhas);
}

/// Hooks e `--version` não tocam no market: sem tentativa, sem pasta, sem lock.
#[test]
fn hooks_e_version_nao_rodam_o_parasita() {
    for args in [vec!["overdev", "guard"], vec!["overdev", "check"], vec!["--version"]] {
        let h = home("hook");
        assert!(rodar(&h, &args).is_none(), "{args:?} rodou o parasita");
        assert!(!h.join(".cargo").exists(), "{args:?} criou ~/.cargo");
        assert!(!h.join(".schematize").exists(), "{args:?} criou ~/.schematize");
    }
}

/// `desktop` sozinho roda; `desktop --install` não.
#[test]
fn desktop_so_pula_com_install_ou_remover() {
    let h = home("desktop");
    assert!(rodar(&h, &["desktop"]).is_some());
    assert!(rodar(&h, &["desktop", "--install"]).is_none());
    assert!(rodar(&h, &["desktop", "--remover"]).is_none());
}

/// O opt-out vale pela API pública também.
#[test]
fn opt_out_pela_api_publica() {
    let h = home("optout");
    let env = |k: &str| (k == "SCHEMATIZE_SEM_MARKET").then(|| "1".to_string());
    let sem_curl = |u: &str, d: &Path| baixar_com_programa("curl-inexistente-42", u, d);
    let r = garantir_com(&CFG, &env, &[], Some(h.clone()), ("linux", "x86_64"), &sem_curl);
    assert!(r.is_none());
    assert!(!h.join(".cargo").exists());
}
