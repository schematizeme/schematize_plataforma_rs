//! `Snapshot` atômico com versão de esquema e checksum, mais backup `.prev`.
//!
//! Layout: `"SCHSNAP1"(8) | esquema u32 | seq u64 | len u64 | dados | crc32 u32` (LE), com o
//! CRC cobrindo tudo antes dele.

use super::crc32::crc32;
use super::erro::{io, Erro};
use super::fsutil::sincroniza_dir;
use std::path::{Path, PathBuf};

const MAGIA: &[u8; 8] = b"SCHSNAP1";
const FIXO: usize = 8 + 4 + 8 + 8;

/// **O quê:** de onde o snapshot carregado veio.
///
/// **Onde:** `Snapshot::carregar`; `Anterior` indica que o último estava ruim (alertar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origem {
    Atual,
    Anterior,
}

/// **O quê:** estado serializado até o seq `seq` (inclusive).
///
/// **Onde:** `Maquina::snapshot` e o boot de `Maquina::abrir`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub versao_esquema: u32,
    pub seq: u64,
    pub dados: Vec<u8>,
}

/// **O quê:** caminho do snapshot atual. **Onde:** `gravar`/`carregar`.
pub fn caminho_atual(dir: &Path) -> PathBuf {
    dir.join("snapshot.bin")
}

/// **O quê:** caminho do backup. **Onde:** `gravar`/`carregar`.
pub fn caminho_prev(dir: &Path) -> PathBuf {
    dir.join("snapshot.prev")
}

impl Snapshot {
    /// **O quê:** serializa com magia e CRC. **Onde:** `gravar`.
    pub fn codificar(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(FIXO + self.dados.len() + 4);
        v.extend_from_slice(MAGIA);
        v.extend_from_slice(&self.versao_esquema.to_le_bytes());
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&(self.dados.len() as u64).to_le_bytes());
        v.extend_from_slice(&self.dados);
        let crc = crc32(&v);
        v.extend_from_slice(&crc.to_le_bytes());
        v
    }

    /// **O quê:** valida e decodifica; nunca entra em pânico com lixo/truncado.
    ///
    /// **Onde:** `carregar`, `gravar` (para saber se o atual vale virar `.prev`) e testes.
    pub fn decodificar(b: &[u8]) -> Result<Snapshot, Erro> {
        let ruim = |m: &str| Erro::SnapshotInvalido(m.to_string());
        if b.len() < FIXO + 4 {
            return Err(ruim("arquivo menor que o cabeçalho"));
        }
        if &b[..8] != MAGIA {
            return Err(ruim("magia incorreta"));
        }
        let versao_esquema = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[12..20]);
        let seq = u64::from_le_bytes(a);
        a.copy_from_slice(&b[20..28]);
        let len = u64::from_le_bytes(a);
        if len != (b.len() - FIXO - 4) as u64 {
            return Err(ruim("tamanho declarado diverge do arquivo"));
        }
        let corpo = b.len() - 4;
        let esperado = u32::from_le_bytes([b[corpo], b[corpo + 1], b[corpo + 2], b[corpo + 3]]);
        if crc32(&b[..corpo]) != esperado {
            return Err(ruim("checksum divergente"));
        }
        Ok(Snapshot { versao_esquema, seq, dados: b[FIXO..corpo].to_vec() })
    }

    /// **O quê:** grava atomicamente (tmp + fsync + rename + fsync do dir). Se já há um snapshot
    /// atual VÁLIDO, ele vira `.prev` antes (um ruim nunca sobrescreve um `.prev` bom).
    ///
    /// **Onde:** `Maquina::snapshot`. Falha (ex.: disco cheio) deixa o estado anterior intacto.
    pub fn gravar(&self, dir: &Path) -> Result<(), Erro> {
        std::fs::create_dir_all(dir).map_err(io(format!("criar {}", dir.display())))?;
        let atual = caminho_atual(dir);
        let novo = self.codificar();
        let tmp = super::fsutil::nome_tmp(&atual);
        escreve_tmp_ou_limpa(&tmp, &novo)?;
        if le_valido(&atual).is_some() {
            std::fs::rename(&atual, caminho_prev(dir)).map_err(io("promover snapshot a .prev"))?;
        }
        std::fs::rename(&tmp, &atual).map_err(io("instalar snapshot novo"))?;
        sincroniza_dir(dir)
    }

    /// **O quê:** carrega o melhor snapshot: atual, senão `.prev`. `Ok(None)` se não há nenhum.
    ///
    /// **Onde:** `Maquina::abrir`. Existindo arquivo(s) e nenhum válido: `SnapshotInvalido`.
    pub fn carregar(dir: &Path) -> Result<Option<(Snapshot, Origem)>, Erro> {
        let (atual, prev) = (caminho_atual(dir), caminho_prev(dir));
        let (ra, rp) = (le_opcional(&atual)?, le_opcional(&prev)?);
        if ra.is_none() && rp.is_none() {
            return Ok(None);
        }
        let ea = match ra.map(|b| Snapshot::decodificar(&b)) {
            Some(Ok(s)) => return Ok(Some((s, Origem::Atual))),
            Some(Err(e)) => e.to_string(),
            None => "ausente".to_string(),
        };
        match rp.map(|b| Snapshot::decodificar(&b)) {
            Some(Ok(s)) => Ok(Some((s, Origem::Anterior))),
            Some(Err(e)) => Err(Erro::SnapshotInvalido(format!("atual: {ea}; prev: {e}"))),
            None => Err(Erro::SnapshotInvalido(format!("atual: {ea}; prev ausente"))),
        }
    }
}

/// **O quê:** escreve o tmp; em erro, remove-o. **Onde:** `Snapshot::gravar`.
fn escreve_tmp_ou_limpa(tmp: &Path, bytes: &[u8]) -> Result<(), Erro> {
    use std::io::Write;
    let r = std::fs::File::create(tmp).map_err(io(format!("criar {}", tmp.display()))).and_then(
        |mut f| {
            f.write_all(bytes).map_err(io("gravar snapshot"))?;
            f.sync_all().map_err(io("fsync do snapshot"))
        },
    );
    if let Err(e) = r {
        return match std::fs::remove_file(tmp) {
            Ok(()) => Err(e),
            Err(r) if r.kind() == std::io::ErrorKind::NotFound => Err(e),
            Err(r) => Err(Erro::Io { contexto: format!("{e}; e não removi o tmp"), fonte: r }),
        };
    }
    Ok(())
}

/// **O quê:** lê o arquivo se existir (`None` se não existe; outros erros propagam).
///
/// **Onde:** `carregar`.
fn le_opcional(p: &Path) -> Result<Option<Vec<u8>>, Erro> {
    match std::fs::read(p) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Erro::Io { contexto: format!("ler {}", p.display()), fonte: e }),
    }
}

/// **O quê:** snapshot do arquivo se existir e for válido. **Onde:** `gravar`.
fn le_valido(p: &Path) -> Option<Snapshot> {
    std::fs::read(p).ok().and_then(|b| Snapshot::decodificar(&b).ok())
}
