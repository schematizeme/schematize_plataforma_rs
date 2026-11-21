//! `Maquina<E>`: snapshot + replay idempotente, fallback `.prev`, snapshot à frente do journal,
//! esquema incompatível, property-style com modelo e kill -9 real com snapshots no caminho.
#![cfg(feature = "persistencia")]

use schematize_plataforma::persistencia::{Config, Durabilidade, Erro, Estado, Maquina, Rng};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Estado de exemplo (contador + checklist): quantos incrementos e quais itens foram feitos.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Lista {
    total: u64,
    feitos: Vec<u32>,
}

/// Evento de exemplo.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ev {
    Inc,
    Feito(u32),
}

impl Estado for Lista {
    type Evento = Ev;
    const VERSAO_ESQUEMA: u32 = 1;

    /// Estado vazio.
    fn inicial() -> Self {
        Lista::default()
    }

    /// Inc soma; Feito acrescenta o item (re-aplicar o mesmo item não duplica).
    fn aplicar(&mut self, e: &Ev) {
        match e {
            Ev::Inc => self.total += 1,
            Ev::Feito(n) if !self.feitos.contains(n) => self.feitos.push(*n),
            Ev::Feito(_) => {}
        }
    }

    /// `total u64 | n u32 | n x u32`.
    fn codificar(&self) -> Vec<u8> {
        let mut v = self.total.to_le_bytes().to_vec();
        v.extend((self.feitos.len() as u32).to_le_bytes());
        for f in &self.feitos {
            v.extend(f.to_le_bytes());
        }
        v
    }

    /// Inverso de `codificar`, validando tamanhos.
    fn decodificar(b: &[u8]) -> Result<Self, String> {
        if b.len() < 12 {
            return Err("curto".into());
        }
        let total = u64::from_le_bytes(b[..8].try_into().map_err(|_| "total")?);
        let n = u32::from_le_bytes(b[8..12].try_into().map_err(|_| "n")?) as usize;
        if b.len() != 12 + 4 * n {
            return Err("tamanho".into());
        }
        let feitos =
            b[12..].chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Ok(Lista { total, feitos })
    }

    /// Inc = `[0]`, Feito = `[1, n]`.
    fn codificar_evento(e: &Ev) -> Vec<u8> {
        match e {
            Ev::Inc => vec![0],
            Ev::Feito(n) => [vec![1], n.to_le_bytes().to_vec()].concat(),
        }
    }

    /// Inverso de `codificar_evento`.
    fn decodificar_evento(b: &[u8]) -> Result<Ev, String> {
        match b {
            [0] => Ok(Ev::Inc),
            [1, a, b2, c, d] => Ok(Ev::Feito(u32::from_le_bytes([*a, *b2, *c, *d]))),
            _ => Err(format!("evento ilegível: {b:?}")),
        }
    }
}

/// Mesmo modelo com outro esquema (para testar incompatibilidade).
#[derive(Default)]
struct ListaV2(Lista);

impl Estado for ListaV2 {
    type Evento = Ev;
    const VERSAO_ESQUEMA: u32 = 2;
    /// Delega.
    fn inicial() -> Self {
        ListaV2::default()
    }
    /// Delega.
    fn aplicar(&mut self, e: &Ev) {
        self.0.aplicar(e)
    }
    /// Delega.
    fn codificar(&self) -> Vec<u8> {
        self.0.codificar()
    }
    /// Delega.
    fn decodificar(b: &[u8]) -> Result<Self, String> {
        Lista::decodificar(b).map(ListaV2)
    }
    /// Delega.
    fn codificar_evento(e: &Ev) -> Vec<u8> {
        Lista::codificar_evento(e)
    }
    /// Delega.
    fn decodificar_evento(b: &[u8]) -> Result<Ev, String> {
        Lista::decodificar_evento(b)
    }
}

/// Config: fsync a cada evento, segmentos pequenos (rotação e compactação frequentes).
fn cfg() -> Config {
    Config { durabilidade: Durabilidade::Cada, tamanho_segmento: 256 }
}

/// **O quê:** pasta única por teste. **Onde:** testes.
fn tmp(nome: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/testes")
        .join(format!("pers-m-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// **O quê:** abre a máquina ou derruba o teste. **Onde:** testes.
fn abre(dir: &Path) -> Maquina<Lista> {
    Maquina::<Lista>::abrir(dir, cfg()).expect("abrir").0
}

/// Retoma exatamente: mesmo estado e seq após reabrir; sem eventos duplicados.
#[test]
fn retoma_estado_exato() {
    let dir = tmp("retoma");
    let mut m = abre(&dir);
    for i in 0..10 {
        m.registrar(&Ev::Inc).expect("inc");
        m.registrar(&Ev::Feito(i)).expect("feito");
    }
    let (esperado, seq) = (m.estado().clone(), m.seq());
    drop(m);
    let m = abre(&dir);
    assert_eq!((m.estado(), m.seq()), (&esperado, seq));
    assert_eq!((m.estado().total, m.estado().feitos.len(), seq), (10, 10, 20));
}

/// `aplicar_seq` com seq repetido ou antigo é no-op.
#[test]
fn reaplicar_mesmo_seq_e_noop() {
    let mut m = abre(&tmp("idem"));
    let s = m.registrar(&Ev::Inc).expect("inc");
    assert!(!m.aplicar_seq(s, &Ev::Inc));
    assert!(!m.aplicar_seq(s - 1, &Ev::Inc));
    assert_eq!(m.estado().total, 1);
    assert!(m.aplicar_seq(s + 1, &Ev::Inc));
    assert_eq!(m.estado().total, 2);
}

/// Snapshot + replay: depois de snapshots o journal compacta e o estado continua exato.
#[test]
fn snapshot_mais_replay_e_compactacao() {
    let dir = tmp("snap");
    let mut m = abre(&dir);
    for _ in 0..30 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("snap1");
    for _ in 0..30 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("snap2");
    for _ in 0..7 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    drop(m);
    let segs = std::fs::read_dir(dir.join(""))
        .expect("dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".wal"))
        .count();
    assert!(segs < 15, "journal deveria ter compactado, segmentos={segs}");
    let m = abre(&dir);
    assert_eq!((m.estado().total, m.seq()), (67, 67));
}

/// Snapshot atual corrompido: usa o `.prev` + replay e avisa no relatório.
#[test]
fn snapshot_ruim_cai_no_prev() {
    let dir = tmp("prev");
    let mut m = abre(&dir);
    for _ in 0..20 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("s1");
    for _ in 0..20 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("s2");
    for _ in 0..5 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    drop(m);
    std::fs::write(dir.join("snapshot.bin"), b"lixo").expect("estragar");
    let (m, rel) = Maquina::<Lista>::abrir(&dir, cfg()).expect("abrir com prev");
    assert_eq!((m.estado().total, m.seq()), (45, 45));
    assert!(rel.snapshot_aviso.is_some_and(|a| a.contains(".prev")));
}

/// Os dois snapshots ruins: replay do zero só se o journal começa no seq 1; senão erro explícito.
#[test]
fn dois_snapshots_ruins() {
    let dir = tmp("ruins");
    let mut m = abre(&dir);
    for _ in 0..5 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("s1");
    drop(m);
    std::fs::write(dir.join("snapshot.bin"), b"x").expect("estragar");
    let (m, rel) = Maquina::<Lista>::abrir(&dir, cfg()).expect("replay do zero");
    assert_eq!(m.estado().total, 5);
    assert!(rel.snapshot_aviso.is_some());
    drop(m);
    let mut m = abre(&dir);
    for _ in 0..200 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("s2");
    m.snapshot().expect("s3 (compacta até o s2)");
    drop(m);
    std::fs::write(dir.join("snapshot.bin"), b"x").expect("estragar");
    std::fs::write(dir.join("snapshot.prev"), b"y").expect("estragar");
    assert!(matches!(Maquina::<Lista>::abrir(&dir, cfg()), Err(Erro::SnapshotInvalido(_))));
}

/// Esquema diferente do snapshot: erro explícito.
#[test]
fn esquema_incompativel() {
    let dir = tmp("esq");
    let mut m = abre(&dir);
    m.registrar(&Ev::Inc).expect("inc");
    m.snapshot().expect("snap");
    drop(m);
    let r = Maquina::<ListaV2>::abrir(&dir, cfg());
    assert!(matches!(r, Err(Erro::EsquemaIncompativel { snapshot: 1, esperado: 2 })));
}

/// Snapshot à frente do journal (cauda do journal perdida): retoma do snapshot e segue.
#[test]
fn snapshot_a_frente_do_journal() {
    let dir = tmp("frente");
    let mut m = abre(&dir);
    for _ in 0..12 {
        m.registrar(&Ev::Inc).expect("inc");
    }
    m.snapshot().expect("snap");
    drop(m);
    for e in std::fs::read_dir(&dir).expect("dir").filter_map(Result::ok) {
        if e.file_name().to_string_lossy().ends_with(".wal") {
            std::fs::remove_file(e.path()).expect("rm");
        }
    }
    let mut m = abre(&dir);
    assert_eq!((m.estado().total, m.seq()), (12, 12));
    assert_eq!(m.registrar(&Ev::Inc).expect("inc"), 13);
    drop(m);
    assert_eq!(abre(&dir).estado().total, 13);
}

/// Property-style: operações aleatórias (eventos, snapshots, reaberturas) contra um modelo.
#[test]
fn property_modelo() {
    let seed =
        std::env::var("PERSIST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
            t.map(|d| d.as_nanos() as u64).unwrap_or(1)
        });
    let mut rng = Rng::novo(seed);
    let dir = tmp("prop");
    let (mut modelo, mut m) = (Lista::default(), abre(&dir));
    for passo in 0..400 {
        match rng.ate(10) {
            0 => m.snapshot().unwrap_or_else(|e| panic!("seed={seed} passo={passo}: {e}")),
            1 => {
                drop(m);
                m = abre(&dir);
            }
            2..=3 => {
                let n = rng.ate(20) as u32;
                m.registrar(&Ev::Feito(n)).expect("feito");
                modelo.aplicar(&Ev::Feito(n));
            }
            _ => {
                m.registrar(&Ev::Inc).expect("inc");
                modelo.aplicar(&Ev::Inc);
            }
        }
        assert_eq!(m.estado(), &modelo, "seed={seed} passo={passo}");
    }
}

/// **Filho** da máquina: registra `Inc` sem parar e tira snapshot a cada 25; imprime `OK <seq>`
/// depois de cada evento confirmado. Só roda com a env do pai (que o mata com SIGKILL).
#[test]
fn filho_maquina() {
    let Ok(dir) = std::env::var("PERS_FILHO_MAQ") else { return };
    let mut m = abre(Path::new(&dir));
    let mut out = std::io::stdout();
    loop {
        let seq = m.registrar(&Ev::Inc).expect("inc");
        writeln!(out, "OK {seq}").expect("stdout");
        out.flush().expect("flush");
        if seq.is_multiple_of(25) {
            m.snapshot().expect("snapshot");
        }
    }
}

/// **O quê:** uma rodada do filho (mata após `ms`); devolve o maior seq confirmado.
///
/// **Onde:** `kill9_com_snapshots`.
fn rodada(dir: &Path, ms: u64) -> u64 {
    let exe = std::env::current_exe().expect("exe");
    let mut f = Command::new(exe)
        .args(["--exact", "filho_maquina", "--nocapture", "--test-threads=1"])
        .env("PERS_FILHO_MAQ", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    std::thread::sleep(Duration::from_millis(ms));
    f.kill().expect("SIGKILL no PID do próprio filho");
    f.wait().expect("wait");
    let saida = f.stdout.take().expect("stdout");
    BufReader::new(saida)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| l.split("OK ").nth(1).and_then(|s| s.trim().parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
}

/// kill -9 real com snapshots e compactação no caminho (morte no meio do snapshot inclusive):
/// o estado retomado é sempre exato (cada evento é `Inc`, logo `total == seq`).
#[test]
fn kill9_com_snapshots() {
    let seed =
        std::env::var("PERSIST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
            t.map(|d| d.as_nanos() as u64).unwrap_or(1)
        });
    let mut rng = Rng::novo(seed);
    let dir = tmp("kill");
    let (mut confirmado, mut anterior) = (0u64, 0u64);
    for it in 0..60 {
        let desta = rodada(&dir, 10 + rng.ate(70));
        confirmado = confirmado.max(desta);
        let m = Maquina::<Lista>::abrir(&dir, cfg())
            .unwrap_or_else(|e| panic!("seed={seed} it={it}: {e}"))
            .0;
        assert_eq!(m.estado().total, m.seq(), "seed={seed} it={it}: estado != seq");
        assert!(m.seq() >= confirmado, "seed={seed} it={it}: perdeu confirmado {confirmado}");
        assert!(m.seq() <= anterior.max(desta) + 1, "seed={seed} it={it}: evento fantasma");
        anterior = m.seq();
    }
    assert!(confirmado > 100, "teste vazio: {confirmado} (seed={seed})");
}
