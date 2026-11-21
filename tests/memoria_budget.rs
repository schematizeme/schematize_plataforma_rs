//! Prova do BUDGET de memória do módulo: Monitor observando 64 alvos fica < 2 MiB de RSS extra.
//!
//! Único teste do arquivo (processo próprio): o pico do processo (`VmHWM`) é zerado via
//! `/proc/self/clear_refs` (valor 5) logo antes de subir o monitor, e comparado ao RSS de base.
#![cfg(feature = "memoria")]

use std::sync::Arc;
use std::time::Duration;

use schematize_plataforma::memoria::{Alvo, ConfigMonitor, FixtureDir, FonteDir, Monitor};

/// Teto do budget (KiB): 2 MiB.
const BUDGET_KIB: u64 = 2048;

fn status_kib(campo: &str) -> Option<u64> {
    let t = std::fs::read_to_string("/proc/self/status").ok()?;
    t.lines().find_map(|l| {
        let r = l.strip_prefix(campo)?.strip_prefix(':')?;
        r.split_whitespace().next()?.parse().ok()
    })
}

fn montar(f: &FixtureDir, n: usize) {
    f.escrever("proc/meminfo", "MemTotal: 16777216 kB\nMemAvailable: 8388608 kB\n").expect("w");
    let psi = "some avg10=1.0 avg60=1.0 avg300=1.0 total=1\nfull avg10=0.5 avg60=0.5 avg300=0.5 total=1\n";
    for i in 0..n {
        let w = |a: &str, c: &str| f.escrever(&format!("sys/fs/cgroup/b{i}/{a}"), c).expect("w");
        w("memory.current", &format!("{}\n", 100_000_000 + i * 1000));
        w("memory.max", "1073741824\n");
        w("memory.high", "max\n");
        w("memory.events", "low 0\nhigh 0\nmax 0\noom 0\noom_kill 0\n");
        w("memory.pressure", psi);
        w("cpu.pressure", psi);
        w("io.pressure", psi);
    }
}

#[test]
fn monitor_com_64_alvos_fica_abaixo_de_2_mib() {
    let f = FixtureDir::nova("budget").expect("fixture");
    montar(&f, 64);
    if let Err(e) = std::fs::write("/proc/self/clear_refs", "5") {
        eprintln!("IGNORADO: não consegui zerar VmHWM (/proc/self/clear_refs): {e}; budget NÃO provado neste ambiente");
        return;
    }
    let base = status_kib("VmRSS").expect("VmRSS");
    let alvos: Vec<Alvo> =
        (0..64).map(|i| Alvo::cgroup(format!("b{i}"), format!("/b{i}"))).collect();
    let cfg = ConfigMonitor { intervalo: Duration::from_millis(250), ..ConfigMonitor::default() };
    let (mon, rx) =
        Monitor::iniciar(&cfg, Arc::new(FonteDir::com_raiz(f.raiz())), alvos).expect("iniciar");
    std::thread::sleep(Duration::from_millis(3000));
    let hwm = status_kib("VmHWM").expect("VmHWM");
    mon.parar().expect("parar");
    drop(rx);
    let extra = hwm.saturating_sub(base);
    eprintln!("budget: base={base} KiB hwm={hwm} KiB extra={extra} KiB (limite {BUDGET_KIB} KiB)");
    assert!(extra < BUDGET_KIB, "monitor com 64 alvos usou {extra} KiB (> {BUDGET_KIB} KiB)");
}
