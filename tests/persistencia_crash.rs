//! kill -9 real do escritor no meio de uma gravação: 200 iterações sobre o MESMO diretório.
//! A cada rodada, a recuperação tem que ser consistente: sem duplicata, sem buraco e sem perder
//! nenhum registro confirmado (fsync) — payload conferido byte a byte.
#![cfg(feature = "persistencia")]

use schematize_plataforma::persistencia::{Config, Durabilidade, Journal, Rng};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// **O quê:** payload determinístico de `seq`: pequeno e pseudo-aleatório, ou (1 a cada 25)
/// um bloco de 64 KB que o SIGKILL pode interromper no meio do `write` (torn write real).
///
/// **Onde:** filho e pai (mesma função).
fn payload(seq: u64) -> Vec<u8> {
    if seq.is_multiple_of(25) {
        return vec![(seq % 251) as u8; 64_000];
    }
    Rng::novo(seq).bytes(1 + (seq as usize * 13) % 90)
}

/// **O quê:** configuração do journal sob teste: fsync a cada registro, segmentos de 4 KiB
/// (muita rotação). **Onde:** filho e pai.
fn cfg() -> Config {
    Config { durabilidade: Durabilidade::Cada, tamanho_segmento: 4096 }
}

/// **Filho** escritor: recupera e anexa sem parar; imprime `OK <seq>` DEPOIS de cada anexo
/// confirmado. Só roda com a env do pai; o pai o mata com SIGKILL.
#[test]
fn filho_escritor() {
    let Ok(dir) = std::env::var("PERS_FILHO_ESC") else { return };
    let mut j = Journal::abrir(Path::new(&dir), cfg()).expect("abrir");
    j.recuperar().expect("recuperar");
    let mut out = std::io::stdout();
    loop {
        let seq = j.proximo_seq();
        j.anexar(1, &payload(seq)).expect("anexar");
        writeln!(out, "OK {seq}").expect("stdout");
        out.flush().expect("flush");
    }
}

/// **O quê:** lança o filho, deixa trabalhar `ms` e o mata com SIGKILL; devolve o maior seq que
/// ele confirmou. **Onde:** laço do teste.
fn rodada(dir: &Path, ms: u64) -> u64 {
    let exe = std::env::current_exe().expect("exe");
    let mut f = Command::new(exe)
        .args(["--exact", "filho_escritor", "--nocapture", "--test-threads=1"])
        .env("PERS_FILHO_ESC", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    std::thread::sleep(Duration::from_millis(ms));
    f.kill().expect("SIGKILL no PID do próprio filho");
    f.wait().expect("wait");
    let saida = f.stdout.take().expect("stdout");
    let mut maior = 0;
    for l in BufReader::new(saida).lines().map_while(Result::ok) {
        // A 1a linha vem colada em "test filho_escritor ... " (libtest): procura o marcador.
        if let Some(n) = l.split("OK ").nth(1).and_then(|s| s.trim().parse::<u64>().ok()) {
            maior = maior.max(n);
        }
    }
    maior
}

/// **O quê:** pasta limpa do teste. **Onde:** o teste.
fn tmp() -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/testes")
        .join(format!("pers-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("criar");
    d
}

/// 200 kill -9 em momentos pseudo-aleatórios (seed na falha): recuperação sempre consistente.
#[test]
fn kill9_200_iteracoes_recuperacao_consistente() {
    let seed =
        std::env::var("PERSIST_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
            t.map(|d| d.as_nanos() as u64).unwrap_or(1)
        });
    let mut rng = Rng::novo(seed);
    let dir = tmp();
    let (mut confirmado, mut caudas_torn, mut anterior) = (0u64, 0u32, 0u64);
    for it in 0..200 {
        let ms = 8 + rng.ate(60);
        let desta = rodada(&dir, ms);
        confirmado = confirmado.max(desta);
        let mut j =
            Journal::abrir(&dir, cfg()).unwrap_or_else(|e| panic!("seed={seed} it={it}: {e}"));
        let (regs, rel) = j.recuperar().unwrap_or_else(|e| panic!("seed={seed} it={it}: {e}"));
        for (i, r) in regs.iter().enumerate() {
            let ok = r.seq == i as u64 + 1 && r.payload == payload(r.seq);
            assert!(ok, "seed={seed} it={it}: registro {i} inconsistente (seq {})", r.seq);
        }
        let ultimo = regs.last().map_or(0, |r| r.seq);
        assert!(
            ultimo >= confirmado,
            "seed={seed} it={it}: perdeu confirmado {confirmado} (último {ultimo})"
        );
        assert!(
            // No máximo 1 anexo não confirmado (o do instante da morte) por rodada.
            ultimo <= anterior.max(desta) + 1,
            "seed={seed} it={it}: apareceu seq {ultimo} nunca anexado"
        );
        anterior = ultimo;
        caudas_torn += u32::from(rel.bytes_descartados > 0);
        j.fechar().expect("fechar");
    }
    assert!(confirmado > 100, "teste vazio: só {confirmado} confirmados (seed={seed})");
    eprintln!("seed={seed} confirmados={confirmado} caudas_torn_recuperadas={caudas_torn}");
}
