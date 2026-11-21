//! Parsers e `Leitura` sobre cgroupfs/proc simulados (inclui arquivos hostis).
#![cfg(feature = "memoria")]

use schematize_plataforma::memoria::parse::{
    parse_io_stat, parse_limite, parse_meminfo, parse_proc_cgroup, parse_psi, parse_u64,
};
use schematize_plataforma::memoria::{FixtureDir, FonteDir, Leitura, Limite};

const CG: &str = "/sys/fs/cgroup/u1";

fn fixture_completa() -> FixtureDir {
    let f = FixtureDir::nova("leitura").expect("fixture");
    let w = |n: &str, c: &str| f.escrever(&format!("sys/fs/cgroup/u1/{n}"), c).expect("escrever");
    w("memory.current", "104857600\n");
    w("memory.peak", "209715200\n");
    w("memory.max", "max\n");
    w("memory.high", "536870912\n");
    w("memory.stat", "anon 1000\nfile 2000\nkernel 300\nslab 40\nshmem 5\nfoo 9\n");
    w("memory.swap.current", "0\n");
    w("memory.events", "low 0\nhigh 3\nmax 2\noom 1\noom_kill 1\n");
    w("pids.current", "7\n");
    w("cpu.stat", "usage_usec 12345\nnr_throttled 4\nthrottled_usec 9\n");
    w("io.stat", "8:0 rbytes=10 wbytes=20 rios=1 wios=2 dbytes=0 dios=0\n8:16 rbytes=5 wbytes=5 rios=1 wios=1\n");
    let psi = "some avg10=1.50 avg60=2.00 avg300=3.00 total=100\nfull avg10=0.50 avg60=0.60 avg300=0.70 total=50\n";
    w("memory.pressure", psi);
    w("cpu.pressure", "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n");
    w("io.pressure", psi);
    f
}

#[test]
fn leitura_completa_le_todos_os_campos() {
    let f = fixture_completa();
    let l = Leitura::de_cgroup(&FonteDir::com_raiz(f.raiz()), CG);
    assert_eq!(l.memoria_atual.v(), Some(104_857_600));
    assert_eq!(l.memoria_pico.v(), Some(209_715_200));
    assert_eq!(l.memoria_max.v(), Some(Limite::Max));
    assert_eq!(l.memoria_high.v(), Some(Limite::Bytes(536_870_912)));
    let s = l.stat.v().expect("stat");
    assert_eq!(
        (s.anon, s.file, s.kernel, s.slab, s.shmem),
        (Some(1000), Some(2000), Some(300), Some(40), Some(5))
    );
    assert_eq!(l.eventos.v().and_then(|e| e.oom_kill), Some(1));
    assert_eq!(l.eventos.v().and_then(|e| e.high), Some(3));
    assert_eq!(l.pids_atual.v(), Some(7));
    assert_eq!(l.cpu.v().and_then(|c| c.nr_throttled), Some(4));
    let io = l.io.v().expect("io");
    assert_eq!((io.rbytes, io.wbytes, io.rios, io.wios, io.dispositivos), (15, 25, 2, 3, 2));
    let full = l.psi_memoria.v().and_then(|p| p.full).expect("full");
    assert_eq!((full.avg10, full.total_us), (0.5, 50));
    assert!(l.psi_cpu.v().and_then(|p| p.full).is_none(), "cpu sem linha full");
    assert!(l.meminfo.valor.is_none() && l.meminfo.motivo.is_some());
}

#[test]
fn arquivos_ausentes_viram_none_com_motivo_sem_panic() {
    let f = FixtureDir::nova("vazia").expect("fixture");
    let l = Leitura::de_cgroup(&FonteDir::com_raiz(f.raiz()), CG);
    assert!(l.memoria_atual.valor.is_none());
    assert!(l.memoria_atual.motivo.as_deref().is_some_and(|m| m.contains("memory.current")));
    assert!(l.psi_memoria.valor.is_none(), "PSI ausente");
    assert!(l.motivos().len() >= 13, "cada campo explica a ausência: {:?}", l.motivos());
}

#[test]
fn arquivos_hostis_e_truncados() {
    let f = FixtureDir::nova("hostil").expect("fixture");
    let w = |n: &str, b: &[u8]| {
        f.escrever_bytes(&format!("sys/fs/cgroup/u1/{n}"), b).expect("escrever")
    };
    w("memory.current", b"\xff\xfe\x00lixo");
    w("memory.max", b"99999999999999999999999999\n");
    w("memory.events", b"oom_kill");
    w("memory.stat", b"anon\nfile abc\nslab 1 2\n");
    w(
        "memory.pressure",
        b"some avg10=nan avg60=0 avg300=0 total=1\nfull avg10=-1 avg60=0 avg300=0 total=1\n",
    );
    w("cpu.pressure", b"some avg10=1.0 avg60=1.0\n");
    w("io.pressure", b"some avg10=250 avg60=0 avg300=0 total=1\n");
    let l = Leitura::de_cgroup(&FonteDir::com_raiz(f.raiz()), CG);
    for (nome, ausente) in [
        ("current", l.memoria_atual.valor.is_none()),
        ("max", l.memoria_max.valor.is_none()),
        ("events", l.eventos.valor.is_none()),
        ("stat", l.stat.valor.is_none()),
        ("psi_mem", l.psi_memoria.valor.is_none()),
        ("psi_cpu", l.psi_cpu.valor.is_none()),
        ("psi_io", l.psi_io.valor.is_none()),
    ] {
        assert!(ausente, "{nome} hostil deveria virar None");
    }
}

#[test]
fn arquivo_gigante_e_limitado() {
    let f = FixtureDir::nova("gigante").expect("fixture");
    let mut b = b"123\n".to_vec();
    b.extend(std::iter::repeat_n(b'x', 2 * 1024 * 1024));
    f.escrever_bytes("sys/fs/cgroup/u1/memory.current", &b).expect("escrever");
    let l = Leitura::de_cgroup(&FonteDir::com_raiz(f.raiz()), CG);
    assert!(l.memoria_atual.valor.is_none(), "lixo depois do número invalida o valor");
}

#[test]
fn fonte_recusa_dot_dot() {
    let f = FixtureDir::nova("dotdot").expect("fixture");
    let l = Leitura::de_cgroup(&FonteDir::com_raiz(f.raiz()), "/sys/fs/cgroup/../../etc");
    assert!(l.memoria_atual.motivo.as_deref().is_some_and(|m| m.contains("..")));
}

#[test]
fn parsers_de_borda() {
    assert_eq!(parse_u64(" 42 \n"), Ok(42));
    assert!(parse_u64("-1").is_err() && parse_u64("").is_err());
    assert_eq!(parse_limite("max\n"), Ok(Limite::Max));
    assert_eq!(parse_limite("0"), Ok(Limite::Bytes(0)));
    assert!(parse_limite("maxx").is_err());
    assert!(parse_io_stat("").expect("vazio é válido").dispositivos == 0);
    assert!(parse_io_stat("lixo sem dispositivo\n").is_err());
    assert!(parse_psi("").is_err());
    let m = parse_meminfo("MemTotal: 16000 kB\nMemAvailable: 8000 kB\n").expect("meminfo");
    assert_eq!((m.total, m.disponivel), (Some(16_384_000), Some(8_192_000)));
    assert!(
        parse_meminfo("MemAvailable: 18446744073709551615 kB\n").is_err(),
        "overflow kB->bytes não vira valor"
    );
    assert_eq!(parse_proc_cgroup("0::/a/b (deleted)\n"), Ok("/a/b".to_string()));
    assert!(parse_proc_cgroup("0::/a/../b\n").is_err());
    assert!(parse_proc_cgroup("1:cpu:/x\n").is_err());
}

#[test]
fn host_le_meminfo_e_pressure() {
    let f = FixtureDir::nova("host").expect("fixture");
    f.escrever("proc/meminfo", "MemTotal: 2048 kB\nMemAvailable: 1024 kB\n").expect("w");
    f.escrever("proc/pressure/memory", "some avg10=0.10 avg60=0.20 avg300=0.30 total=7\n")
        .expect("w");
    let l = Leitura::do_host(&FonteDir::com_raiz(f.raiz()));
    assert_eq!(l.origem, "host");
    assert_eq!(l.meminfo.v().and_then(|m| m.disponivel), Some(1_048_576));
    assert_eq!(l.psi_memoria.v().and_then(|p| p.some).map(|s| s.total_us), Some(7));
    assert!(l.psi_io.valor.is_none(), "PSI de io ausente com motivo");
}
