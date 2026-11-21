//! Observador/Monitor sobre cgroupfs simulado: bordas, histerese, contadores que regridem.
#![cfg(feature = "memoria")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use schematize_plataforma::memoria::anel::Anel;
use schematize_plataforma::memoria::{
    derivada_mib_s, Alvo, ConfigMonitor, Evento, FixtureDir, FonteDir, Monitor, Observador, Recurso,
};

const MIB: u64 = 1024 * 1024;

struct Mundo {
    f: FixtureDir,
    fonte: FonteDir,
}

fn mundo() -> Mundo {
    let f = FixtureDir::nova("monitor").expect("fixture");
    f.escrever("proc/meminfo", "MemTotal: 16777216 kB\nMemAvailable: 8388608 kB\n").expect("w");
    let fonte = FonteDir::com_raiz(f.raiz());
    Mundo { f, fonte }
}

impl Mundo {
    fn cg(&self, nome: &str, arq: &str, c: &str) {
        self.f.escrever(&format!("sys/fs/cgroup/{nome}/{arq}"), c).expect("escrever");
    }

    fn estado(&self, nome: &str, atual: u64, high: &str, events: &str) {
        self.cg(nome, "memory.current", &format!("{atual}\n"));
        self.cg(nome, "memory.high", &format!("{high}\n"));
        self.cg(nome, "memory.max", &format!("{}\n", 1024 * MIB));
        self.cg(nome, "memory.events", events);
    }
}

fn ev(high: u64, oom_kill: u64) -> String {
    format!("low 0\nhigh {high}\nmax 0\noom 0\noom_kill {oom_kill}\n")
}

fn obs(alvos: Vec<Alvo>) -> Observador {
    Observador::novo(&ConfigMonitor::default(), alvos)
}

#[test]
fn cruzou_high_na_borda_com_histerese() {
    let m = mundo();
    let cfg = ConfigMonitor { crescimento_mib_s: 1e9, ..ConfigMonitor::default() };
    let mut o = Observador::novo(&cfg, vec![Alvo::cgroup("u", "/u")]);
    m.estado("u", 100 * MIB, "500000000", &ev(0, 0));
    assert!(o.amostrar(&m.fonte, 0).is_empty());
    m.estado("u", 600 * MIB, "500000000", &ev(0, 0));
    let e = o.amostrar(&m.fonte, 1000);
    assert!(
        matches!(e.as_slice(), [Evento::CruzouHigh { atual_bytes, .. }] if *atual_bytes == 600 * MIB),
        "{e:?}"
    );
    assert!(o.amostrar(&m.fonte, 2000).is_empty(), "não repete enquanto acima");
    m.estado("u", 100 * MIB, "500000000", &ev(0, 0));
    assert!(o.amostrar(&m.fonte, 3000).is_empty());
    m.estado("u", 600 * MIB, "500000000", &ev(0, 0));
    assert_eq!(o.amostrar(&m.fonte, 4000).len(), 1, "rearmou abaixo de 90% do high");
}

#[test]
fn high_max_literal_so_dispara_por_contador() {
    let m = mundo();
    let mut o = obs(vec![Alvo::cgroup("u", "/u")]);
    m.estado("u", 900 * MIB, "max", &ev(0, 0));
    assert!(o.amostrar(&m.fonte, 0).is_empty(), "high=max e sem delta: nada");
    m.estado("u", 900 * MIB, "max", &ev(2, 0));
    let e = o.amostrar(&m.fonte, 1000);
    assert!(
        matches!(e.as_slice(), [Evento::CruzouHigh { high_bytes: None, delta_events_high: 2, .. }]),
        "{e:?}"
    );
}

#[test]
fn oom_kill_delta_e_contador_que_regride() {
    let m = mundo();
    let mut o = obs(vec![Alvo::cgroup("u", "/u")]);
    m.estado("u", MIB, "max", &ev(0, 5));
    assert!(o.amostrar(&m.fonte, 0).is_empty(), "1ª leitura só fixa a base (kill antigo)");
    m.estado("u", MIB, "max", &ev(0, 7));
    let e = o.amostrar(&m.fonte, 1000);
    assert_eq!(e, vec![Evento::OomKillOcorreu { alvo: "u".into(), delta: 2, total: 7 }]);
    m.estado("u", MIB, "max", &ev(0, 1));
    assert!(o.amostrar(&m.fonte, 2000).is_empty(), "regressão não gera evento nem delta negativo");
    m.estado("u", MIB, "max", &ev(0, 2));
    assert!(matches!(
        o.amostrar(&m.fonte, 3000).as_slice(),
        [Evento::OomKillOcorreu { delta: 1, .. }]
    ));
}

#[test]
fn psi_full_acima_do_limiar_e_ausente() {
    let m = mundo();
    let mut o = obs(vec![Alvo::cgroup("u", "/u")]);
    m.estado("u", MIB, "max", &ev(0, 0));
    assert!(o.amostrar(&m.fonte, 0).is_empty(), "sem arquivos PSI: sem evento e sem panic");
    let psi = |x: f64| {
        format!(
            "some avg10=1.0 avg60=0 avg300=0 total=1\nfull avg10={x} avg60=0 avg300=0 total=1\n"
        )
    };
    m.cg("u", "memory.pressure", &psi(25.0));
    let e = o.amostrar(&m.fonte, 1000);
    assert!(
        matches!(e.as_slice(), [Evento::PressaoPsi { recurso: Recurso::Memoria, full_avg10, .. }] if *full_avg10 == 25.0),
        "{e:?}"
    );
    assert!(o.amostrar(&m.fonte, 2000).is_empty());
    m.cg("u", "memory.pressure", &psi(1.0));
    assert!(o.amostrar(&m.fonte, 3000).is_empty());
    m.cg("u", "memory.pressure", &psi(30.0));
    assert_eq!(o.amostrar(&m.fonte, 4000).len(), 1);
}

#[test]
fn crescimento_rapido_projeta_tempo_ate_o_teto() {
    let m = mundo();
    let mut o = obs(vec![Alvo::cgroup("u", "/u")]);
    let mut vistos = Vec::new();
    for i in 0..5u64 {
        m.estado("u", (100 + i * 100) * MIB, "max", &ev(0, 0));
        vistos.extend(o.amostrar(&m.fonte, i * 1000));
    }
    let [Evento::CrescimentoRapido { mib_por_s, segundos_ate_teto, teto_bytes, .. }] =
        vistos.as_slice()
    else {
        panic!("esperava 1 CrescimentoRapido, veio {vistos:?}");
    };
    assert!((mib_por_s - 100.0).abs() < 1e-6, "{mib_por_s}");
    assert_eq!(*teto_bytes, Some(1024 * MIB));
    let s = segundos_ate_teto.expect("projeção");
    assert!(s > 5.0 && s < 8.0, "(1024-300)/100 ~ 7,2 s, veio {s}");
}

#[test]
fn derivada_com_poucos_pontos_ou_tempo_parado() {
    let mut a = Anel::novo(8);
    assert_eq!(derivada_mib_s(&a), None);
    a.push((0, 0));
    a.push((0, MIB));
    a.push((0, 2 * MIB));
    assert_eq!(derivada_mib_s(&a), None, "sem avanço de tempo");
}

#[test]
fn anel_sobrescreve_o_mais_antigo() {
    let mut a = Anel::novo(3);
    for i in 0..5u64 {
        a.push(i);
    }
    assert_eq!((a.len(), a.get(0), a.ultimo()), (3, Some(2), Some(4)));
    a.limpar();
    assert!(a.is_empty());
}

#[test]
fn host_apertado_na_borda_e_falha_so_quando_muda() {
    let m = mundo();
    let cfg = ConfigMonitor { margem_host_mib: 1024, ..ConfigMonitor::default() };
    let mut o = Observador::novo(&cfg, vec![Alvo::pid("p", 999_999)]);
    let e = o.amostrar(&m.fonte, 0);
    assert!(
        matches!(e.as_slice(), [Evento::FalhaLeitura { alvo, .. }] if alvo == "p"),
        "pid inexistente: {e:?}"
    );
    assert!(o.amostrar(&m.fonte, 1000).is_empty(), "mesmo motivo não repete");
    m.f.escrever("proc/meminfo", "MemAvailable: 524288 kB\n").expect("w");
    let e = o.amostrar(&m.fonte, 2000);
    assert!(
        matches!(e.as_slice(), [Evento::HostApertado { disponivel_bytes, margem_bytes }] if *disponivel_bytes == 512 * MIB && *margem_bytes == 1024 * MIB),
        "{e:?}"
    );
    assert!(o.amostrar(&m.fonte, 3000).is_empty());
    m.f.escrever("proc/meminfo", "MemAvailable: 4194304 kB\n").expect("w");
    assert!(o.amostrar(&m.fonte, 4000).is_empty());
    m.f.escrever("proc/meminfo", "MemAvailable: 524288 kB\n").expect("w");
    assert_eq!(o.amostrar(&m.fonte, 5000).len(), 1, "rearmou");
    m.f.remover("proc/meminfo").expect("rm");
    let e = o.amostrar(&m.fonte, 6000);
    assert!(e.iter().any(|x| matches!(x, Evento::FalhaLeitura { alvo, .. } if alvo == "host")));
}

#[test]
fn pid_resolve_o_cgroup_e_recusa_dot_dot() {
    let m = mundo();
    m.f.escrever("proc/42/cgroup", "0::/x/y.scope\n").expect("w");
    m.estado("x/y.scope", MIB, "max", &ev(0, 0));
    let mut o = obs(vec![Alvo::pid("p", 42), Alvo::cgroup("mau", "/a/../../etc")]);
    let e = o.amostrar(&m.fonte, 0);
    assert!(e.iter().any(|x| matches!(x, Evento::FalhaLeitura { alvo, motivo } if alvo == "mau" && motivo.contains(".."))), "{e:?}");
    assert!(
        !e.iter().any(|x| matches!(x, Evento::FalhaLeitura { alvo, .. } if alvo == "p")),
        "{e:?}"
    );
}

#[test]
fn definir_alvos_mantem_estado_dos_iguais() {
    let m = mundo();
    let mut o = obs(vec![Alvo::cgroup("u", "/u")]);
    m.estado("u", MIB, "max", &ev(0, 5));
    o.amostrar(&m.fonte, 0);
    o.definir_alvos(vec![Alvo::cgroup("u", "/u"), Alvo::cgroup("v", "/v")]);
    assert_eq!(o.n_alvos(), 2);
    m.estado("u", MIB, "max", &ev(0, 6));
    let e = o.amostrar(&m.fonte, 1000);
    assert!(
        e.iter().any(|x| matches!(x, Evento::OomKillOcorreu { delta: 1, .. })),
        "base de u preservada: {e:?}"
    );
}

fn espera_evento(rx: &std::sync::mpsc::Receiver<Evento>, ate: Duration) -> Option<Evento> {
    rx.recv_timeout(ate).ok()
}

#[test]
fn monitor_thread_emite_e_nunca_bloqueia() {
    let m = mundo();
    m.estado("u", MIB, "max", &ev(0, 0));
    let cfg = ConfigMonitor {
        intervalo: Duration::from_millis(250),
        capacidade_canal: 1,
        ..ConfigMonitor::default()
    };
    let fonte = Arc::new(FonteDir::com_raiz(m.f.raiz()));
    let (mon, rx) = Monitor::iniciar(&cfg, fonte, vec![Alvo::cgroup("u", "/u")]).expect("iniciar");
    std::thread::sleep(Duration::from_millis(400));
    m.estado("u", MIB, "max", &ev(0, 3));
    let e = espera_evento(&rx, Duration::from_secs(3)).expect("evento OOM");
    assert!(matches!(e, Evento::OomKillOcorreu { delta: 3, .. }), "{e:?}");
    mon.definir_alvos(vec![Alvo::cgroup("fantasma", "/nao-existe")]);
    let e = espera_evento(&rx, Duration::from_secs(3)).expect("falha do alvo novo");
    assert!(matches!(e, Evento::FalhaLeitura { .. }), "{e:?}");
    let t = Instant::now();
    mon.parar().expect("parar");
    assert!(t.elapsed() < Duration::from_millis(200), "parar acorda o thread na hora");
}

#[test]
fn monitor_canal_cheio_descarta_e_conta() {
    let m = mundo();
    let cfg = ConfigMonitor {
        intervalo: Duration::from_millis(250),
        capacidade_canal: 1,
        ..ConfigMonitor::default()
    };
    let alvos = (0..4).map(|i| Alvo::cgroup(format!("n{i}"), format!("/ausente{i}"))).collect();
    let (mon, _rx) =
        Monitor::iniciar(&cfg, Arc::new(FonteDir::com_raiz(m.f.raiz())), alvos).expect("iniciar");
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        mon.descartados() >= 3,
        "4 falhas em canal de 1 => >= 3 descartes, veio {}",
        mon.descartados()
    );
    mon.parar().expect("parar");
}

#[test]
fn intervalo_e_clampado() {
    let c = ConfigMonitor {
        intervalo: Duration::from_millis(1),
        capacidade_anel: 0,
        psi_full_limiar: f64::NAN,
        ..ConfigMonitor::default()
    };
    let n = c.normalizada();
    assert_eq!(n.intervalo, Duration::from_millis(250));
    assert_eq!(n.capacidade_anel, 4);
    assert_eq!(n.psi_full_limiar, 10.0);
    let c = ConfigMonitor { intervalo: Duration::from_secs(60), ..ConfigMonitor::default() };
    assert_eq!(c.normalizada().intervalo, Duration::from_secs(5));
}
