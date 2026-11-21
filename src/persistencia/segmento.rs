//! Segmentos do WAL no disco: nomes `seg-<primeiro seq, 20 dígitos>.wal`, listagem e varredura.

use super::erro::{io, Erro};
use super::registro::{ler_um, Leitura, Registro};
use std::path::{Path, PathBuf};

/// **O quê:** nome do arquivo de segmento cujo primeiro registro terá seq `primeiro`.
///
/// **Onde:** rotação e criação do journal.
pub fn nome(primeiro: u64) -> String {
    format!("seg-{primeiro:020}.wal")
}

/// **O quê:** extrai o primeiro seq do nome de um segmento; `None` se não for segmento.
///
/// **Onde:** `listar`.
fn primeiro_do_nome(nome: &str) -> Option<u64> {
    nome.strip_prefix("seg-")?.strip_suffix(".wal")?.parse().ok()
}

/// **O quê:** lista os segmentos de `dir` ordenados pelo primeiro seq.
///
/// **Onde:** recuperação e compactação. Ignora arquivos que não são segmentos (lock, tmp).
pub fn listar(dir: &Path) -> Result<Vec<(u64, PathBuf)>, Erro> {
    let ctx = format!("listar {}", dir.display());
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).map_err(io(ctx.clone()))? {
        let e = e.map_err(io(ctx.clone()))?;
        let nome = e.file_name();
        if let Some(p) = nome.to_str().and_then(primeiro_do_nome) {
            v.push((p, e.path()));
        }
    }
    v.sort_by_key(|(p, _)| *p);
    Ok(v)
}

/// **O quê:** resultado da varredura de um segmento.
///
/// **Onde:** `Journal::recuperar_desde`.
#[derive(Debug)]
pub struct Varredura {
    /// Registros íntegros e contíguos a partir do seq inicial.
    pub registros: Vec<Registro>,
    /// Bytes que compõem o prefixo íntegro.
    pub bytes_ok: usize,
    /// Por que a varredura parou antes do fim (cauda ruim), se parou.
    pub ruim: Option<String>,
}

/// **O quê:** varre `buf` registro a registro exigindo seq contíguo a partir de `esperado`.
///
/// **Onde:** recuperação. CRC ok mas seq fora de ordem é `BuracoDeSeq` (erro explícito, não
/// truncamento); cauda parcial/CRC ruim só encerra a varredura com o motivo.
pub fn varrer(buf: &[u8], esperado: u64, seg: &Path) -> Result<Varredura, Erro> {
    let (mut off, mut prox) = (0usize, esperado);
    let mut registros = Vec::new();
    loop {
        match ler_um(&buf[off..]) {
            Leitura::Fim => return Ok(Varredura { registros, bytes_ok: off, ruim: None }),
            Leitura::Ruim(m) => return Ok(Varredura { registros, bytes_ok: off, ruim: Some(m) }),
            Leitura::Ok(r, n) => {
                if r.seq != prox {
                    let (esperado, encontrado, segmento) = (prox, r.seq, seg.to_path_buf());
                    return Err(Erro::BuracoDeSeq { esperado, encontrado, segmento });
                }
                prox += 1;
                off += n;
                registros.push(r);
            }
        }
    }
}
