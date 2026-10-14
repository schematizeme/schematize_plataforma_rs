//! Testes das decisões puras do parasita: o que falta, URLs, quando roda, hooks, mensagens.
//!
//! **Onde:** `cargo test`; nenhum toca o HOME real nem a rede.

use super::*;
use std::fs;

/// **O quê:** pasta temporária única por teste (sem crate externo).
///
/// **Onde:** os testes do parasita.
pub(super) fn tmp(nome: &str) -> PathBuf {
    let d = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/target/testes"))
        .join(format!("plat-parasita-{}-{nome}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// **O quê:** ambiente vazio.
///
/// **Onde:** os testes do parasita.
pub(super) fn sem_env(_: &str) -> Option<String> {
    None
}

/// **O quê:** `Vec<String>` a partir de literais.
///
/// **Onde:** os testes de `deve_rodar`.
fn v(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Config com os hooks do overdev (como cli/overdev usam).
const CFG_HOOKS: Config<'static> = Config::new("schematize").com_hooks(HOOKS_OVERDEV);

#[test]
fn falta_os_dois_um_ou_nenhum() {
    let d = tmp("falta");
    assert_eq!(o_que_falta(&d), NOMES.to_vec());
    fs::write(d.join(nome_do_arquivo(NOMES[0], std::env::consts::OS)), "x").unwrap();
    assert_eq!(o_que_falta(&d), vec![NOMES[1]]);
    fs::write(d.join(nome_do_arquivo(NOMES[1], std::env::consts::OS)), "x").unwrap();
    assert!(o_que_falta(&d).is_empty());
}

#[test]
fn windows_procura_o_exe() {
    let d = tmp("winexe");
    fs::write(d.join(NOMES[0]), "x").unwrap();
    assert_eq!(o_que_falta_para(&d, "windows"), NOMES.to_vec(), "sem .exe não conta");
    fs::write(d.join("schematize-market.exe"), "x").unwrap();
    assert_eq!(o_que_falta_para(&d, "windows"), vec![NOMES[1]]);
}

#[test]
fn url_dos_alvos_publicados() {
    let base = "https://github.com/schematizeme/schematize_market_rs/releases/latest/download/";
    let u = url_do_asset;
    let esp = [
        ("schematize-market", "linux", "x86_64", "schematize-market-linux-x86_64"),
        ("schematize-market", "macos", "aarch64", "schematize-market-macos-arm64"),
        ("schematize-market", "macos", "arm64", "schematize-market-macos-arm64"),
        ("schematize-market-gui", "macos", "x86_64", "schematize-market-gui-macos-x86_64"),
        ("schematize-market-gui", "windows", "x86_64", "schematize-market-gui-windows-x86_64.exe"),
        ("schematize-market", "freebsd", "x86_64", "schematize-market-freebsd-x86_64"),
        ("schematize-market-gui", "freebsd", "x86_64", "schematize-market-gui-freebsd-x86_64"),
    ];
    for (n, o, a, fim) in esp {
        assert_eq!(u(n, o, a), Some(format!("{base}{fim}")), "{n} {o} {a}");
    }
}

#[test]
fn plataformas_sem_asset() {
    for (o, a) in [
        ("linux", "aarch64"),
        ("linux", "arm64"),
        ("freebsd", "aarch64"),
        ("windows", "aarch64"),
        ("linux", "riscv64"),
        ("linux", ""),
        ("haiku", "x86_64"),
        ("", "x86_64"),
        ("Linux", "x86_64"),
    ] {
        assert_eq!(url_do_asset("schematize-market", o, a), None, "{o}/{a}");
    }
}

#[test]
fn plataforma_atual_mapeia_aarch64_para_arm64() {
    let (os, arch) = plataforma_atual();
    assert_eq!(os, std::env::consts::OS);
    assert_ne!(arch, "aarch64", "aarch64 tem de virar arm64");
    if std::env::consts::ARCH == "aarch64" {
        assert_eq!(arch, "arm64");
    }
}

#[test]
fn opt_out_por_ambiente() {
    for var in ["SCHEMATIZE_SEM_MARKET", "SCHEMATIZE_DELEGADO_AO_MARKET"] {
        let env = move |k: &str| (k == var).then(|| "1".to_string());
        assert!(!deve_rodar(&CFG_HOOKS, &env, &[]), "{var}");
        // Valor diferente de "1" NÃO desliga (só o contrato "1").
        let zero = move |k: &str| (k == var).then(|| "0".to_string());
        assert!(deve_rodar(&CFG_HOOKS, &zero, &[]), "{var}=0 não é opt-out");
    }
    let ci = |k: &str| (k == "CI").then(String::new);
    assert!(!deve_rodar(&CFG_HOOKS, &ci, &[]), "CI definida (até vazia) desliga");
}

#[test]
fn nao_roda_em_version_help_desktop_e_hooks() {
    for args in [
        v(&["--version"]),
        v(&["-V"]),
        v(&["--help"]),
        v(&["-h"]),
        v(&["help"]),
        v(&["lang", "--version"]),
        v(&["desktop", "--install"]),
        v(&["desktop", "--remover"]),
        v(&["overdev", "guard"]),
        v(&["overdev", "check"]),
        v(&["guard"]),
        v(&["check", "extra"]),
        v(&["hooks"]),
    ] {
        assert!(!deve_rodar(&CFG_HOOKS, &sem_env, &args), "{args:?}");
    }
    for args in [v(&[]), v(&["status"]), v(&["overdev", "status"]), v(&["--dir", "/p"])] {
        assert!(deve_rodar(&CFG_HOOKS, &sem_env, &args), "{args:?}");
    }
}

#[test]
fn desktop_sozinho_roda_so_install_e_remover_pulam() {
    for args in [v(&["desktop"]), v(&["desktop", "--status"]), v(&["desktop", "--dir", "x"])] {
        assert!(deve_rodar(&CFG_HOOKS, &sem_env, &args), "{args:?} deve rodar");
    }
    assert!(!deve_rodar(&CFG_HOOKS, &sem_env, &v(&["desktop", "x", "--install"])));
    // `--install` fora do `desktop` não pula.
    assert!(deve_rodar(&CFG_HOOKS, &sem_env, &v(&["lang", "--install"])));
}

#[test]
fn hooks_sao_parametro_do_host() {
    let sem = Config::new("otimizador");
    // Sem padrões, `overdev guard` e `guard` NÃO são hooks.
    assert!(deve_rodar(&sem, &sem_env, &v(&["overdev", "guard"])));
    assert!(deve_rodar(&sem, &sem_env, &v(&["guard"])));
    // Padrão próprio do host.
    let meu = Config::new("x").com_hooks(&[&["gancho", "pre|pos"], &["tick"]]);
    for args in [v(&["gancho", "pre"]), v(&["gancho", "pos", "z"]), v(&["tick"])] {
        assert!(!deve_rodar(&meu, &sem_env, &args), "{args:?}");
    }
    for args in [v(&["gancho"]), v(&["gancho", "meio"]), v(&["ticket"]), v(&["overdev", "guard"])] {
        assert!(deve_rodar(&meu, &sem_env, &args), "{args:?}");
    }
}

#[test]
fn padrao_de_hook_vazio_nao_casa_tudo() {
    let c = Config::new("x").com_hooks(&[&[]]);
    assert!(deve_rodar(&c, &sem_env, &v(&["qualquer"])));
    assert!(deve_rodar(&c, &sem_env, &[]));
}

#[test]
fn args_hostis_nao_derrubam_a_decisao() {
    for a in ["", "\0", "日本語", "--version\u{200b}", "  ", "--", "-"] {
        let _ = deve_rodar(&CFG_HOOKS, &sem_env, &v(&[a]));
        let _ = deve_rodar(&CFG_HOOKS, &sem_env, &v(&["desktop", a]));
    }
    assert!(deve_rodar(&CFG_HOOKS, &sem_env, &v(&["--VERSION"])), "case-sensitive");
}

#[test]
fn linhas_seguem_o_contrato_e_listam_cada_falha() {
    let c = Config::new("meu-app");
    let pasta = Path::new("/p");
    let mut r = Resultado { instalados: vec![NOMES[0]], ..Resultado::default() };
    r.falhas.push((NOMES[1], "causa um".into()));
    r.falhas.push(("outro", "causa dois".into()));
    r.avisos.push("lock preso".into());
    let l = linhas(&c, &r, pasta);
    assert_eq!(l.len(), 4, "{l:?}");
    assert_eq!(l[0], "meu-app: instalei o gestor de atualizações (schematize-market) em /p");
    assert!(l[1].starts_with("meu-app: não consegui instalar o schematize-market-gui (causa um)"));
    assert!(l[1].contains("o app segue normal") && l[1].contains(INSTALAR));
    assert!(l[2].contains("instalar o outro (causa dois)"));
    assert_eq!(l[3], "meu-app: aviso: lock preso");
    assert!(linhas(&c, &Resultado::default(), pasta).is_empty(), "silêncio");
}

#[test]
fn emitir_devolve_o_erro_de_escrita_em_vez_de_panico() {
    /// Escritor que sempre falha (stderr fechado).
    struct Quebrado;
    impl Write for Quebrado {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("fechado"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(emitir(&["a".into()], &mut Quebrado).is_err());
    let mut buf = Vec::new();
    emitir(&["a".into(), "b".into()], &mut buf).unwrap();
    assert_eq!(String::from_utf8(buf).unwrap(), "a\nb\n");
}

#[test]
fn caminho_novo_por_plataforma() {
    assert_eq!(caminho_novo(Path::new("/b/x"), "linux"), PathBuf::from("/b/x.novo"));
    assert_eq!(caminho_novo(Path::new("/b/x.exe"), "windows"), PathBuf::from("/b/x.novo.exe"));
    assert_eq!(caminho_novo(Path::new("/b/x"), "windows"), PathBuf::from("/b/x.novo.exe"));
}

#[test]
fn home_hostil_vira_pasta_sem_panico() {
    // Cadeia HOME/USERPROFILE: vazio, unicode, espaços (usado por `garantir`).
    let amb = |h: &'static str| move |k: &str| (k == "HOME").then(|| h.to_string());
    assert_eq!(home_de_teste(&amb("")), None);
    assert_eq!(home_de_teste(&amb("/h ç/日本 x")), Some(PathBuf::from("/h ç/日本 x")));
}

/// **O quê:** atalho para a cadeia de HOME da lib.
///
/// **Onde:** `home_hostil_vira_pasta_sem_panico`.
fn home_de_teste(get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    crate::home::home_de(get)
}
