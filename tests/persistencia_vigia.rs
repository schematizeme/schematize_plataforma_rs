//! `Vigia`: relança filho que morre por SIGKILL/exit, abre o circuito (parked), registra as
//! mortes no próprio journal, auto-proteção de memória (VmHWM < 5 MiB) e unit systemd.
#![cfg(feature = "persistencia")]

use schematize_plataforma::persistencia::{
    rodar, unit_systemd, Config, Desfecho, Erro, Journal, VigiaCfg,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// **O quê:** pasta única por teste. **Onde:** testes.
fn tmp(nome: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/testes")
        .join(format!("pers-v-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("criar");
    d
}

/// **O quê:** config do vigia para um `sh -c <script>` com backoff mínimo.
///
/// **Onde:** testes. O `--resume` do vigia vira o `$0` do script.
fn cfg(dir: &Path, script: &str, max: u32) -> VigiaCfg {
    let mut c = VigiaCfg::novo("/bin/sh", dir.join("estado"));
    c.args = vec!["-c".into(), script.into()];
    c.max_mortes = max;
    c.backoff_base = Duration::from_millis(1);
    c.backoff_teto = Duration::from_millis(5);
    c
}

/// **O quê:** linhas (texto) do journal do vigia. **Onde:** testes.
fn diario(c: &VigiaCfg) -> Vec<String> {
    let mut j = Journal::abrir(&c.dir_estado.join("vigia"), Config::default()).expect("abrir");
    let (regs, _) = j.recuperar().expect("recuperar");
    regs.iter().map(|r| String::from_utf8_lossy(&r.payload).into_owned()).collect()
}

/// Filho que se mata com SIGKILL N vezes: relançado com `--resume`, depois o circuito abre,
/// grava `parked`, para, e um novo vigia se recusa a rodar enquanto o marcador existir.
#[test]
fn sigkill_n_vezes_depois_abre_o_circuito() {
    let dir = tmp("circuito");
    let log = dir.join("args.log");
    let script = format!("echo \"$0\" >> {}; kill -9 $$", log.display());
    let c = cfg(&dir, &script, 3);
    assert_eq!(rodar(&c).expect("rodar"), Desfecho::Parked { mortes: 3 });
    assert!(c.dir_estado.join("parked").exists());
    let args = std::fs::read_to_string(&log).expect("log");
    assert_eq!(args.lines().collect::<Vec<_>>(), ["/bin/sh", "--resume", "--resume"]);
    let d = diario(&c);
    let mortes: Vec<_> = d.iter().filter(|l| l.starts_with("morte ")).collect();
    assert_eq!(mortes.len(), 3, "{d:?}");
    assert!(mortes.iter().all(|m| m.contains("sinal=9") && m.contains("causa=sigkill")));
    assert!(
        d.iter().any(|l| l.contains("resume=false")) && d.iter().any(|l| l.contains("resume=true"))
    );
    assert_eq!(rodar(&c).expect("rodar de novo"), Desfecho::JaParked);
    assert_eq!(args.lines().count(), 3, "JaParked não lança nada");
}

/// Morre 2 vezes e na 3a termina com sucesso: o vigia relança e conclui sem abrir o circuito.
#[test]
fn relanca_ate_concluir() {
    let dir = tmp("conclui");
    let cont = dir.join("cont");
    let script = format!(
        "n=$(cat {c} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {c}; \
         [ $n -ge 3 ] && exit 0; [ $n -eq 1 ] && exit 7; kill -9 $$",
        c = cont.display()
    );
    let c = cfg(&dir, &script, 5);
    assert_eq!(rodar(&c).expect("rodar"), Desfecho::Concluido { lancamentos: 3 });
    assert!(!c.dir_estado.join("parked").exists());
    let d = diario(&c);
    assert!(d.iter().any(|l| l.contains("codigo=7") && l.contains("causa=exit_7")), "{d:?}");
    assert!(d.iter().any(|l| l.contains("sinal=9")));
    assert!(d.iter().any(|l| l.starts_with("fim_ok")));
}

/// Comando inexistente: conta como morte (`falha_spawn`) e o circuito abre, sem loop infinito.
#[test]
fn comando_inexistente_abre_o_circuito() {
    let dir = tmp("spawn");
    let mut c = cfg(&dir, "x", 3);
    c.comando = PathBuf::from("/nao/existe/orquestrador");
    assert_eq!(rodar(&c).expect("rodar"), Desfecho::Parked { mortes: 3 });
}

/// Contador `oom_kill` do cgroup que sobe durante a vida do filho: causa provável = OOM.
#[test]
fn causa_oom_pelo_memory_events() {
    let dir = tmp("oom");
    let ev = dir.join("memory.events");
    std::fs::write(&ev, "oom 0\noom_kill 0\n").expect("fake");
    let script = format!("sleep 0.3; echo 'oom_kill 1' > {}; kill -9 $$", ev.display());
    let mut c = cfg(&dir, &script, 1);
    c.memory_events = Some(ev);
    assert_eq!(rodar(&c).expect("rodar"), Desfecho::Parked { mortes: 1 });
    assert!(diario(&c).iter().any(|l| l.contains("causa=oom_kill_cgroup")));
}

/// Dois vigias no mesmo estado: o segundo recebe `LockOcupado`, nunca dois orquestradores.
#[test]
fn dois_vigias_no_mesmo_estado() {
    let dir = tmp("dois");
    let c = cfg(&dir, "sleep 1", 3);
    let c2 = c.clone();
    let t = std::thread::spawn(move || rodar(&c2));
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(rodar(&c), Err(Erro::LockOcupado { .. })));
    assert_eq!(t.join().expect("thread").expect("rodar"), Desfecho::Concluido { lancamentos: 1 });
}

/// A unit systemd é só texto, com as diretivas pedidas.
#[test]
fn unit_systemd_so_texto() {
    let mut c = VigiaCfg::novo("/opt/orq/overdev", "/var/lib/vigia");
    c.args = vec!["--modo".into(), "x y".into()];
    let u = unit_systemd(&c);
    for d in
        ["Restart=always", "OOMPolicy=continue", "MemoryMax=", "ExecStart=", "\"x y\"", ".slice"]
    {
        assert!(u.contains(d), "faltou {d}");
    }
}

/// **Filho** vigia: roda `rodar` sobre `sleep 2`. Só roda com a env do pai, que mede o VmHWM.
#[test]
fn filho_vigia() {
    let Ok(dir) = std::env::var("PERS_FILHO_VIGIA") else { return };
    let mut c = VigiaCfg::novo("/bin/sleep", &dir);
    c.args = vec!["2".into()];
    let r = rodar(&c).expect("rodar");
    assert_eq!(r, Desfecho::Concluido { lancamentos: 1 });
}

/// **O quê:** `VmHWM` (kB) do processo `pid`. **Onde:** teste de auto-proteção.
fn vmhwm_kb(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("VmHWM:")?.trim().trim_end_matches("kB").trim().parse().ok())
}

/// Auto-proteção: o processo do vigia (rodando `rodar` de verdade) fica abaixo de 5 MiB de RSS
/// de pico (VmHWM) mesmo depois de lançar e esperar o orquestrador.
#[test]
fn vigia_rss_menor_que_5_mib() {
    let dir = tmp("rss");
    let exe = std::env::current_exe().expect("exe");
    let mut f = Command::new(exe)
        .args(["--exact", "filho_vigia", "--nocapture", "--test-threads=1"])
        .env("PERS_FILHO_VIGIA", &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    std::thread::sleep(Duration::from_millis(1000));
    let pico = vmhwm_kb(f.id()).expect("VmHWM legível");
    assert!(f.wait().expect("wait").success());
    assert!(pico < 5 * 1024, "VmHWM do vigia = {pico} kB (teto 5120 kB)");
}
