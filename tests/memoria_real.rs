//! Teste REAL: filho aloca 300 MiB num scope com teto 256 MiB; o Monitor deve ver
//! `CruzouHigh` e/ou `OomKillOcorreu`. Sem `systemd-run --user --scope` utilizável, o teste se
//! auto-ignora imprimindo o motivo (nunca passa em silêncio).
#![cfg(feature = "memoria")]

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use schematize_plataforma::memoria::{Alvo, ConfigMonitor, Evento, FonteDir, Monitor};

const VAR_FILHO: &str = "MEMORIA_FILHO_MIB";

/// Modo filho: quando a variável está setada, aloca e toca a memória e dorme; senão não faz nada.
#[test]
fn filho_aloca() {
    let Ok(v) = std::env::var(VAR_FILHO) else { return };
    let mib: usize = v.parse().expect("MEMORIA_FILHO_MIB numérico");
    let mut blocos: Vec<Vec<u8>> = Vec::new();
    for i in 0..mib {
        blocos.push(vec![(i % 251) as u8 + 1; 1024 * 1024]);
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_secs(5));
    std::hint::black_box(&blocos);
}

fn scope_disponivel() -> Result<(), String> {
    let o = Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "-p", "MemoryMax=64M", "--", "true"])
        .output()
        .map_err(|e| format!("systemd-run indisponível: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "systemd-run --user --scope falhou: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    Ok(())
}

fn lancar_filho() -> std::io::Result<Child> {
    let exe = std::env::current_exe()?;
    Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--quiet",
            "-p",
            "MemoryMax=256M",
            "-p",
            "MemoryHigh=200M",
            "-p",
            "MemorySwapMax=0",
            "--",
        ])
        .arg(exe)
        .args(["--exact", "filho_aloca", "--nocapture", "--test-threads=1"])
        .env(VAR_FILHO, "300")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[test]
fn monitor_ve_high_ou_oom_num_scope_real() {
    if let Err(motivo) = scope_disponivel() {
        eprintln!("IGNORADO: teste real de cgroup não executado: {motivo}");
        return;
    }
    let mut filho = lancar_filho().expect("lançar filho");
    let cfg = ConfigMonitor { intervalo: Duration::from_millis(250), ..ConfigMonitor::default() };
    let alvos = vec![Alvo::pid("filho", filho.id())];
    let (mon, rx) = Monitor::iniciar(&cfg, Arc::new(FonteDir::real()), alvos).expect("iniciar");
    let (mut high, mut oom, mut cresc) = (false, false, false);
    let fim = Instant::now() + Duration::from_secs(20);
    let mut visto_high: Option<Instant> = None;
    // Sob MemoryHigh sem swap o filho pode encalhar (throttle) sem nunca morrer: depois de ver
    // `high`, damos 6 s para o OOM aparecer e então encerramos o filho nós mesmos.
    while Instant::now() < fim
        && !oom
        && visto_high.is_none_or(|t| t.elapsed() < Duration::from_secs(6))
    {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Evento::CruzouHigh { .. }) => {
                high = true;
                visto_high.get_or_insert_with(Instant::now);
            }
            Ok(Evento::OomKillOcorreu { .. }) => oom = true,
            Ok(Evento::CrescimentoRapido { .. }) => cresc = true,
            Ok(outro) => eprintln!("evento: {outro:?}"),
            Err(_) if filho.try_wait().expect("try_wait").is_some() => break,
            Err(_) => {}
        }
    }
    mon.parar().expect("parar");
    if filho.try_wait().expect("try_wait").is_none() {
        filho.kill().expect("encerrar o filho encalhado");
    }
    let status = filho.wait().expect("wait");
    eprintln!("real: high={high} oom={oom} crescimento={cresc} status_filho={status}");
    assert!(high || oom, "o monitor não viu CruzouHigh nem OomKillOcorreu (filho: {status})");
}
