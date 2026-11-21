//! Ponta a ponta do binário `schematize-vigia`: exit codes, circuito, `--status`, unit, uso.
#![cfg(feature = "persistencia")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// **O quê:** pasta única por teste. **Onde:** testes.
fn tmp(nome: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/testes")
        .join(format!("vigia-bin-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("criar");
    d
}

/// **O quê:** roda o binário com `args` (idioma fixo em português). **Onde:** testes.
fn vigia(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_schematize-vigia"))
        .env("SCHEMATIZE_LANG", "pt")
        .args(args)
        .output()
        .expect("executar o vigia")
}

/// **O quê:** stdout como texto. **Onde:** testes.
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Filho que morre por SIGKILL N vezes: circuito abre, exit 3, `--status` mostra, nova rodada
/// também dá 3, e `--limpar-parked` exige `--sim`.
#[test]
fn sigkill_abre_circuito_exit_3_e_status() {
    let d = tmp("circuito");
    let e = d.join("estado");
    let es = e.to_str().expect("utf8");
    let r = vigia(&[
        "--estado",
        es,
        "--max-mortes",
        "3",
        "--backoff-ms",
        "1",
        "--",
        "sh",
        "-c",
        "kill -9 $$",
    ]);
    assert_eq!(r.status.code(), Some(3), "{}", String::from_utf8_lossy(&r.stderr));
    let s = vigia(&["--estado", es, "--status"]);
    assert_eq!(s.status.code(), Some(3));
    let t = out(&s);
    assert!(t.contains("mortes: 3") && t.contains("parked: sim"), "{t}");
    assert!(t.contains("sigkill_sem_oom_visivel") || t.contains("oom_kill_cgroup"), "{t}");
    let de_novo = vigia(&["--estado", es, "--", "sh", "-c", "exit 0"]);
    assert_eq!(de_novo.status.code(), Some(3), "parked persiste entre rodadas");
    assert_eq!(vigia(&["--estado", es, "--limpar-parked"]).status.code(), Some(2));
    assert!(e.join("parked").exists());
    assert_eq!(vigia(&["--estado", es, "--limpar-parked", "--sim"]).status.code(), Some(0));
    let ok = vigia(&["--estado", es, "--", "sh", "-c", "exit 0"]);
    assert_eq!(ok.status.code(), Some(0));
}

/// Filho que sai 0: exit 0 e nenhum parked.
#[test]
fn filho_que_conclui_da_exit_0() {
    let d = tmp("ok");
    let es = d.join("estado");
    let r = vigia(&["--estado", es.to_str().expect("utf8"), "--", "sh", "-c", "exit 0"]);
    assert_eq!(r.status.code(), Some(0));
    assert!(!es.join("parked").exists());
    let s = vigia(&["--estado", es.to_str().expect("utf8"), "--status"]);
    assert!(out(&s).contains("concluído"), "{}", out(&s));
}

/// `--unit-systemd` imprime a unit com o comando e não cria nada em disco.
#[test]
fn unit_systemd_so_imprime() {
    let d = tmp("unit");
    let es = d.join("naoexiste");
    let r = vigia(&["--estado", es.to_str().expect("utf8"), "--unit-systemd", "--", "orq", "a b"]);
    assert_eq!(r.status.code(), Some(0));
    let u = out(&r);
    assert!(u.contains("Restart=always") && u.contains("\"a b\"") && u.contains("--estado"), "{u}");
    assert!(!es.exists(), "nada pode ser criado");
}

/// Uso incorreto = exit 2 com mensagem; `--help`/`--version` = 0; inglês via SCHEMATIZE_LANG.
#[test]
fn uso_ajuda_versao() {
    assert_eq!(vigia(&[]).status.code(), Some(2));
    assert_eq!(vigia(&["--estado", "/x"]).status.code(), Some(2), "sem comando");
    assert_eq!(vigia(&["--bogus"]).status.code(), Some(2));
    assert_eq!(vigia(&["--help"]).status.code(), Some(0));
    assert!(out(&vigia(&["--version"])).contains(env!("CARGO_PKG_VERSION")));
    let en = Command::new(env!("CARGO_BIN_EXE_schematize-vigia"))
        .env("SCHEMATIZE_LANG", "en")
        .arg("--bogus")
        .output()
        .expect("executar");
    assert!(String::from_utf8_lossy(&en.stderr).contains("unknown option"));
}
