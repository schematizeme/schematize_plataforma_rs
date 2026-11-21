//! Testes severos do `Journal` e do `Snapshot`: cauda truncada, bit-flip, lixo, lock, permissão,
//! disco cheio (RLIMIT_FSIZE em processo filho real).
#![cfg(feature = "persistencia")]

use schematize_plataforma::persistencia::registro::codificar;
use schematize_plataforma::persistencia::{Config, Durabilidade, Erro, Journal, Rng, Snapshot};
use std::path::{Path, PathBuf};
use std::process::Command;

/// **O quê:** pasta única por teste sob `target/testes`. **Onde:** todos os testes.
fn tmp(nome: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/testes")
        .join(format!("pers-j-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("criar pasta de teste");
    d
}

/// **O quê:** seed da rodada (env `PERSIST_SEED` ou relógio), impressa para reproduzir.
///
/// **Onde:** testes property-style.
fn seed() -> u64 {
    std::env::var("PERSIST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        t.map(|d| d.as_nanos() as u64).unwrap_or(1)
    })
}

/// **O quê:** config sem fsync (rápida) com segmento de `seg` bytes. **Onde:** testes.
fn cfg_rapida(seg: u64) -> Config {
    Config { durabilidade: Durabilidade::Nunca, tamanho_segmento: seg }
}

/// **O quê:** abre + recupera, falhando o teste com contexto. **Onde:** testes.
fn abre(dir: &Path, cfg: Config) -> Journal {
    let mut j = Journal::abrir(dir, cfg).expect("abrir");
    j.recuperar().expect("recuperar");
    j
}

/// **O quê:** payload determinístico de `seq`. **Onde:** testes.
fn payload(seq: u64) -> Vec<u8> {
    Rng::novo(seq).bytes(1 + (seq as usize * 7) % 40)
}

/// Ida e volta com rotação, reabertura e compactação a partir de um seq.
#[test]
fn roundtrip_rotacao_e_compactacao() {
    let dir = tmp("rot");
    let mut j = abre(&dir, cfg_rapida(300));
    for s in 1..=100u64 {
        assert_eq!(j.anexar(1, &payload(s)).expect("anexar"), s);
    }
    j.fechar().expect("fechar");
    let mut j = Journal::abrir(&dir, cfg_rapida(300)).expect("reabrir");
    let (regs, rel) = j.recuperar().expect("recuperar");
    assert_eq!((regs.len(), rel.validos, rel.bytes_descartados), (100, 100, 0));
    assert!(rel.segmentos > 3, "esperava rotação, segmentos={}", rel.segmentos);
    assert!(regs
        .iter()
        .enumerate()
        .all(|(i, r)| r.seq == i as u64 + 1 && r.payload == payload(r.seq)));
    let removidos = j.compactar(60).expect("compactar");
    assert!(removidos > 0);
    let (cauda, rel) = j.recuperar_desde(61).expect("recuperar desde");
    assert_eq!(cauda.first().map(|r| r.seq), Some(61));
    assert_eq!(rel.ultimo_seq, Some(100));
    assert!(rel.primeiro_seq.is_some_and(|p| p <= 61));
}

/// Monta um único segmento com 6 registros e devolve (bytes, fim de cada registro).
///
/// **Onde:** testes de truncamento e bit-flip.
fn segmento_de_6() -> (Vec<u8>, Vec<usize>) {
    let (mut bytes, mut fins) = (Vec::new(), Vec::new());
    for s in 1..=6u64 {
        bytes.extend(codificar(s, 1, &payload(s)));
        fins.push(bytes.len());
    }
    (bytes, fins)
}

/// Nome do primeiro segmento. **Onde:** testes que fabricam arquivos à mão.
const SEG1: &str = "seg-00000000000000000001.wal";

/// Cauda truncada em TODOS os offsets: sempre o maior prefixo íntegro, resto descartado e o
/// journal volta a aceitar anexos com seq contíguo.
#[test]
fn cauda_truncada_em_todos_os_offsets() {
    let (bytes, fins) = segmento_de_6();
    for corte in 0..=bytes.len() {
        let dir = tmp("trunc");
        std::fs::write(dir.join(SEG1), &bytes[..corte]).expect("escrever");
        let mut j = Journal::abrir(&dir, cfg_rapida(1 << 20)).expect("abrir");
        let (regs, rel) = j.recuperar().expect("recuperar nunca falha com cauda parcial");
        let validos = fins.iter().filter(|&&f| f <= corte).count();
        let fim_ok = if validos == 0 { 0 } else { fins[validos - 1] };
        assert_eq!(regs.len(), validos, "corte={corte}");
        assert_eq!(rel.bytes_descartados, (corte - fim_ok) as u64, "corte={corte}");
        assert_eq!(rel.motivo.is_some(), corte != fim_ok, "corte={corte}");
        assert_eq!(std::fs::metadata(dir.join(SEG1)).expect("meta").len(), fim_ok as u64);
        assert_eq!(j.anexar(1, b"novo").expect("anexar pós-recuperação"), validos as u64 + 1);
        j.fechar().expect("fechar");
        let (regs2, _) = abre_leitura(&dir);
        assert_eq!(regs2.len(), validos + 1, "corte={corte}");
    }
}

/// **O quê:** reabre e recupera devolvendo (registros, relatório). **Onde:** testes.
fn abre_leitura(dir: &Path) -> (Vec<schematize_plataforma::persistencia::Registro>, usize) {
    let mut j = Journal::abrir(dir, cfg_rapida(1 << 20)).expect("abrir");
    let (r, rel) = j.recuperar().expect("recuperar");
    (r, rel.segmentos)
}

/// Bit-flip em cada bit de cada byte de um registro do meio e do último: nunca pânico, nunca
/// aceita o registro adulterado, descarta dali em diante.
#[test]
fn bitflip_em_cada_byte_do_registro() {
    let (bytes, fins) = segmento_de_6();
    for (idx, esperados) in [(2usize, 2usize), (5, 5)] {
        let (ini, fim) = (fins[idx - 1], fins[idx]);
        for pos in ini..fim {
            for bit in 0..8 {
                let dir = tmp("flip");
                let mut b = bytes.clone();
                b[pos] ^= 1 << bit;
                std::fs::write(dir.join(SEG1), &b).expect("escrever");
                let mut j = Journal::abrir(&dir, cfg_rapida(1 << 20)).expect("abrir");
                let (regs, rel) = j.recuperar().expect("recuperar");
                assert_eq!(regs.len(), esperados, "idx={idx} pos={pos} bit={bit}");
                assert_eq!(rel.bytes_descartados, (bytes.len() - ini) as u64);
            }
        }
    }
}

/// Segmento vazio, zerado, lixo aleatório (seed registrada) e `len` hostil: sempre recupera.
#[test]
fn vazio_zerado_lixo_e_len_hostil() {
    let s = seed();
    let mut rng = Rng::novo(s);
    let mut casos: Vec<Vec<u8>> = vec![vec![], vec![0; 4096], vec![0xFF; 64]];
    let mut hostil = vec![0xFF, 0xFF, 0xFF, 0xFF];
    hostil.extend(vec![0u8; 40]);
    casos.push(hostil);
    for _ in 0..200 {
        let n = rng.ate(300) as usize;
        casos.push(rng.bytes(n));
    }
    for (i, c) in casos.iter().enumerate() {
        let dir = tmp("lixo");
        std::fs::write(dir.join(SEG1), c).expect("escrever");
        let mut j = Journal::abrir(&dir, cfg_rapida(1 << 20)).expect("abrir");
        let (regs, _) = j.recuperar().unwrap_or_else(|e| panic!("seed={s} caso={i}: {e}"));
        assert!(regs.is_empty(), "seed={s} caso={i}: lixo virou registro");
        assert_eq!(j.anexar(1, b"x").expect("anexar"), 1, "seed={s} caso={i}");
    }
}

/// Buraco de seq (segmento do meio sumiu) e corrupção em segmento não-final: erros explícitos
/// e NADA é apagado.
#[test]
fn buraco_e_corrupcao_no_meio_sao_erros_explicitos() {
    let dir = tmp("buraco");
    let mut j = abre(&dir, cfg_rapida(200));
    for s in 1..=40u64 {
        j.anexar(1, &payload(s)).expect("anexar");
    }
    j.fechar().expect("fechar");
    let segs = segs_em(&dir);
    assert!(segs.len() >= 4);
    let corrompido = std::fs::read(&segs[1]).map(|mut b| {
        b[20] ^= 0xFF;
        b
    });
    let original = std::fs::read(&segs[1]).expect("ler");
    std::fs::write(&segs[1], corrompido.expect("ler")).expect("escrever");
    let e = Journal::abrir(&dir, cfg_rapida(200)).expect("abrir").recuperar().unwrap_err();
    assert!(matches!(e, Erro::CorrupcaoNoMeio { .. }), "{e}");
    assert_eq!(segs_em(&dir).len(), segs.len(), "nada pode ser apagado");
    std::fs::write(&segs[1], &original).expect("restaurar");
    std::fs::remove_file(&segs[1]).expect("remover");
    let e = Journal::abrir(&dir, cfg_rapida(200)).expect("abrir").recuperar().unwrap_err();
    assert!(matches!(e, Erro::BuracoDeSeq { .. }), "{e}");
    assert_eq!(segs_em(&dir).len(), segs.len() - 1);
}

/// **O quê:** segmentos de `dir` em ordem. **Onde:** testes.
fn segs_em(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("ler dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wal"))
        .collect();
    v.sort();
    v
}

/// Dois escritores: o segundo recebe `LockOcupado` com o pid do dono; lock de pid morto é
/// retomado e sinalizado como órfão.
#[test]
fn dois_escritores_e_lock_orfao() {
    let dir = tmp("lock");
    let j1 = Journal::abrir(&dir, Config::default()).expect("abrir");
    match Journal::abrir(&dir, Config::default()) {
        Err(Erro::LockOcupado { pid, .. }) => assert_eq!(pid, Some(std::process::id())),
        Err(e) => panic!("erro errado: {e}"),
        Ok(_) => panic!("segundo escritor não pode abrir"),
    }
    drop(j1);
    let mut filho = Command::new("/bin/true").spawn().expect("spawn");
    let morto = filho.id();
    filho.wait().expect("wait");
    std::fs::write(dir.join("journal.lock"), morto.to_string()).expect("lock falso");
    let mut j = Journal::abrir(&dir, Config::default()).expect("pid morto não segura o lock");
    let (_, rel) = j.recuperar().expect("recuperar");
    assert!(rel.lock_orfao, "devia sinalizar lock órfão");
}

/// Diretório sem permissão: erro `Io` legível, nunca pânico.
#[test]
fn diretorio_sem_permissao() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp("perm");
    let alvo = dir.join("fechado");
    std::fs::create_dir_all(&alvo).expect("criar");
    std::fs::set_permissions(&alvo, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let root = std::fs::read_dir(&alvo).is_ok();
    let r = Journal::abrir(&alvo.join("sub"), Config::default());
    std::fs::set_permissions(&alvo, std::fs::Permissions::from_mode(0o755)).expect("restaurar");
    if !root {
        assert!(matches!(r, Err(Erro::Io { .. })), "esperava Io");
    }
}

#[repr(C)]
struct Rlim {
    cur: u64,
    max: u64,
}
extern "C" {
    fn setrlimit(recurso: i32, lim: *const Rlim) -> i32;
    fn signal(sig: i32, h: usize) -> usize;
}

/// **Filho** do teste de disco cheio: sob RLIMIT_FSIZE=1000 anexa até falhar e imprime o
/// resultado; depois grava dois snapshots (o segundo estoura). Só roda com a env do pai.
#[test]
fn filho_disco_cheio() {
    let Ok(dir) = std::env::var("PERS_FILHO_FSIZE") else { return };
    let dir = PathBuf::from(dir);
    let lim = Rlim { cur: 1000, max: 1000 };
    // SAFETY: setrlimit/signal com argumentos válidos; SIG_IGN (1) evita morrer por SIGXFSZ.
    let (r, _) = unsafe { (setrlimit(1, &lim), signal(25, 1)) };
    assert_eq!(r, 0, "setrlimit");
    let mut j = abre(&dir.join("j"), Config::default());
    let mut acks = 0u64;
    let erro = loop {
        match j.anexar(1, &[7u8; 40]) {
            Ok(_) => acks += 1,
            Err(e) => break e,
        }
    };
    let de_novo = j.anexar(1, &[7u8; 40]).is_err();
    let snap = dir.join("s");
    Snapshot { versao_esquema: 1, seq: 1, dados: vec![1; 100] }.gravar(&snap).expect("snap 1");
    let grande = Snapshot { versao_esquema: 1, seq: 2, dados: vec![2; 5000] }.gravar(&snap);
    let ok = matches!(Snapshot::carregar(&snap), Ok(Some((s, _))) if s.seq == 1);
    println!(
        "RES acks={acks} io={} de_novo_erro={de_novo} snap_falhou={} snap_intacto={ok}",
        matches!(erro, Erro::Io { .. }),
        grande.is_err()
    );
}

/// Disco cheio simulado: erro propagado, nada pela metade no journal, snapshot anterior intacto.
#[test]
fn disco_cheio_propaga_erro_e_preserva_estado() {
    let dir = tmp("cheio");
    let exe = std::env::current_exe().expect("exe");
    let out = Command::new(exe)
        .args(["--exact", "filho_disco_cheio", "--nocapture", "--test-threads=1"])
        .env("PERS_FILHO_FSIZE", &dir)
        .output()
        .expect("filho");
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    let linha = so
        .lines()
        .find_map(|l| l.find("RES ").map(|i| &l[i..]))
        .unwrap_or_else(|| panic!("{so}{}", String::from_utf8_lossy(&out.stderr)));
    assert!(linha.contains("io=true") && linha.contains("de_novo_erro=true"), "{linha}");
    assert!(linha.contains("snap_falhou=true") && linha.contains("snap_intacto=true"), "{linha}");
    let acks: u64 = linha
        .split("acks=")
        .nth(1)
        .and_then(|s| s.split(' ').next())
        .and_then(|s| s.parse().ok())
        .expect("acks");
    assert!(acks > 3);
    let (regs, rel) = {
        let mut j = Journal::abrir(&dir.join("j"), Config::default()).expect("abrir");
        j.recuperar().expect("recuperar")
    };
    assert_eq!(regs.len() as u64, acks, "só os confirmados sobrevivem");
    assert_eq!(rel.bytes_descartados, 0, "rollback deixou a cauda limpa");
    let sobras = std::fs::read_dir(dir.join("s"))
        .expect("dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .count();
    assert_eq!(sobras, 0, "tmp órfão do snapshot");
}

/// Snapshot: ida e volta, `.prev` quando o último está ruim, lixo/truncado nunca pânico.
#[test]
fn snapshot_prev_e_hostil() {
    let dir = tmp("snap");
    assert!(Snapshot::carregar(&dir).expect("vazio").is_none());
    let (a, b) = (
        Snapshot { versao_esquema: 3, seq: 10, dados: b"um".to_vec() },
        Snapshot { versao_esquema: 3, seq: 20, dados: b"dois".to_vec() },
    );
    a.gravar(&dir).expect("a");
    b.gravar(&dir).expect("b");
    let (s, o) = Snapshot::carregar(&dir).expect("carregar").expect("existe");
    assert_eq!((s.seq, o), (20, schematize_plataforma::persistencia::Origem::Atual));
    let bytes = std::fs::read(dir.join("snapshot.bin")).expect("ler");
    for corte in 0..bytes.len() {
        assert!(Snapshot::decodificar(&bytes[..corte]).is_err(), "corte={corte}");
    }
    for pos in 0..bytes.len() {
        let mut m = bytes.clone();
        m[pos] ^= 0x10;
        assert!(Snapshot::decodificar(&m).is_err(), "pos={pos}");
    }
    std::fs::write(dir.join("snapshot.bin"), &bytes[..bytes.len() - 3]).expect("estragar");
    let (s, o) = Snapshot::carregar(&dir).expect("carregar").expect("prev");
    assert_eq!((s.seq, o), (10, schematize_plataforma::persistencia::Origem::Anterior));
    std::fs::write(dir.join("snapshot.prev"), b"lixo").expect("estragar prev");
    assert!(matches!(Snapshot::carregar(&dir), Err(Erro::SnapshotInvalido(_))));
}

/// Property-style: sequência aleatória de anexos/reaberturas/truncamentos de cauda contra um
/// modelo em memória. A seed vai na mensagem de falha.
#[test]
fn property_modelo_contra_journal() {
    let s = seed();
    let mut rng = Rng::novo(s);
    let dir = tmp("prop");
    let mut modelo: Vec<Vec<u8>> = Vec::new();
    for passo in 0..60 {
        let mut j = Journal::abrir(&dir, cfg_rapida(150 + rng.ate(300))).expect("abrir");
        let (regs, _) = j.recuperar().unwrap_or_else(|e| panic!("seed={s} passo={passo}: {e}"));
        let lidos: Vec<Vec<u8>> = regs.iter().map(|r| r.payload.clone()).collect();
        assert_eq!(lidos, modelo, "seed={s} passo={passo}");
        for _ in 0..rng.ate(15) {
            let tam = rng.ate(60) as usize;
            let p = rng.bytes(tam);
            j.anexar(2, &p).unwrap_or_else(|e| panic!("seed={s}: {e}"));
            modelo.push(p);
        }
        j.fechar().expect("fechar");
        if rng.ate(3) == 0 {
            suja_cauda(&dir, &mut rng, s);
        }
    }
}

/// **O quê:** apende lixo parcial ao último segmento (cauda torn) (só acrescenta bytes: o modelo
/// não muda e a recuperação tem que descartá-los). **Onde:** property test.
fn suja_cauda(dir: &Path, rng: &mut Rng, s: u64) {
    let ultimo = segs_em(dir).pop().unwrap_or_else(|| panic!("seed={s}: sem segmento"));
    let mut b = std::fs::read(&ultimo).expect("ler");
    let k = 1 + rng.ate(20) as usize;
    b.extend(rng.bytes(k));
    std::fs::write(&ultimo, b).expect("escrever");
}
