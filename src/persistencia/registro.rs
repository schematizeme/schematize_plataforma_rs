//! Formato do registro do WAL: `[len u32][crc32 u32][seq u64][tipo u8][payload]` (little-endian).
//!
//! `len` é o tamanho do payload. O CRC cobre `len`, `seq`, `tipo` e `payload`: um bit-flip em
//! qualquer byte do registro (inclusive no próprio `len`) é detectado.

use super::crc32::Crc32;

/// Bytes do cabeçalho: len(4) + crc(4) + seq(8) + tipo(1).
pub const CABECALHO: usize = 17;
/// Teto de payload por registro (protege contra `len` hostil).
pub const MAX_PAYLOAD: usize = 64 << 20;

/// **O quê:** um registro lógico do journal.
///
/// **Onde:** devolvido por `Journal::recuperar`; entrada de replay das máquinas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registro {
    pub seq: u64,
    pub tipo: u8,
    pub payload: Vec<u8>,
}

/// **O quê:** resultado de tentar ler um registro do início de um buffer.
///
/// **Onde:** a varredura de recuperação.
#[derive(Debug, PartialEq, Eq)]
pub enum Leitura {
    /// Registro íntegro e quantos bytes ocupou.
    Ok(Registro, usize),
    /// Fim limpo do buffer (zero bytes restantes).
    Fim,
    /// Cauda parcial ou inválida; o motivo explica.
    Ruim(String),
}

/// **O quê:** CRC do registro a partir dos campos.
///
/// **Onde:** `codificar` e `ler_um`.
fn crc_do_registro(len: [u8; 4], seq: u64, tipo: u8, payload: &[u8]) -> u32 {
    Crc32::default()
        .atualizar(&len)
        .atualizar(&seq.to_le_bytes())
        .atualizar(&[tipo])
        .atualizar(payload)
        .fim()
}

/// **O quê:** serializa um registro em bytes.
///
/// **Onde:** `Journal::anexar` (uma única `write` por registro, para minimizar torn write).
pub fn codificar(seq: u64, tipo: u8, payload: &[u8]) -> Vec<u8> {
    let len = (payload.len() as u32).to_le_bytes();
    let crc = crc_do_registro(len, seq, tipo, payload);
    let mut v = Vec::with_capacity(CABECALHO + payload.len());
    v.extend_from_slice(&len);
    v.extend_from_slice(&crc.to_le_bytes());
    v.extend_from_slice(&seq.to_le_bytes());
    v.push(tipo);
    v.extend_from_slice(payload);
    v
}

/// **O quê:** lê u32 LE do início de `b` (precisa de >= 4 bytes). **Onde:** `ler_um`.
fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// **O quê:** lê u64 LE do início de `b` (precisa de >= 8 bytes). **Onde:** `ler_um`.
fn u64_le(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

/// **O quê:** tenta ler um registro no início de `buf`, sem nunca entrar em pânico.
///
/// **Onde:** laço de varredura de `Journal::recuperar`. Rejeita: cabeçalho incompleto, `len`
/// acima do teto ou além do buffer, CRC divergente e `seq == 0`.
pub fn ler_um(buf: &[u8]) -> Leitura {
    if buf.is_empty() {
        return Leitura::Fim;
    }
    if buf.len() < CABECALHO {
        return Leitura::Ruim(format!("cabeçalho parcial ({} de {CABECALHO} bytes)", buf.len()));
    }
    let len_b = [buf[0], buf[1], buf[2], buf[3]];
    let len = u32_le(&buf[0..4]) as usize;
    if len > MAX_PAYLOAD {
        return Leitura::Ruim(format!("len {len} acima do teto {MAX_PAYLOAD}"));
    }
    if buf.len() - CABECALHO < len {
        return Leitura::Ruim(format!(
            "payload parcial ({} de {len} bytes)",
            buf.len() - CABECALHO
        ));
    }
    let (crc, seq, tipo) = (u32_le(&buf[4..8]), u64_le(&buf[8..16]), buf[16]);
    let payload = &buf[CABECALHO..CABECALHO + len];
    if crc_do_registro(len_b, seq, tipo, payload) != crc {
        return Leitura::Ruim("crc divergente".to_string());
    }
    if seq == 0 {
        return Leitura::Ruim("seq zero é inválido".to_string());
    }
    Leitura::Ok(Registro { seq, tipo, payload: payload.to_vec() }, CABECALHO + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ida e volta, e todo prefixo próprio de um registro é "ruim", nunca pânico.
    #[test]
    fn roundtrip_e_prefixos() {
        let b = codificar(5, 2, b"abc");
        assert_eq!(
            ler_um(&b),
            Leitura::Ok(Registro { seq: 5, tipo: 2, payload: b"abc".to_vec() }, b.len())
        );
        for n in 1..b.len() {
            assert!(matches!(ler_um(&b[..n]), Leitura::Ruim(_)), "prefixo {n}");
        }
        assert_eq!(ler_um(&[]), Leitura::Fim);
        assert!(matches!(ler_um(&[0u8; 64]), Leitura::Ruim(_)));
    }
}
