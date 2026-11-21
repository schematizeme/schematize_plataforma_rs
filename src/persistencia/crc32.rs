//! CRC-32 (IEEE 802.3, polinômio refletido 0xEDB88320) próprio, por tabela. Sem dependências.

/// Tabela de 256 entradas, calculada em tempo de compilação.
const TABELA: [u32; 256] = gera_tabela();

/// **O quê:** gera a tabela do CRC-32 IEEE.
///
/// **Onde:** inicialização da constante [`TABELA`] (compile-time).
const fn gera_tabela() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

/// **O quê:** acumulador incremental de CRC-32.
///
/// **Onde:** checksum de registros do journal e de snapshots.
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

impl Default for Crc32 {
    /// **O quê:** estado inicial. **Onde:** `Crc32::default()`.
    fn default() -> Self {
        Crc32(0xFFFF_FFFF)
    }
}

impl Crc32 {
    /// **O quê:** incorpora `dados` ao checksum. **Onde:** encadeado por quem monta registros.
    pub fn atualizar(mut self, dados: &[u8]) -> Self {
        for &b in dados {
            self.0 = TABELA[((self.0 ^ b as u32) & 0xFF) as usize] ^ (self.0 >> 8);
        }
        self
    }

    /// **O quê:** valor final do checksum. **Onde:** depois do último `atualizar`.
    pub fn fim(self) -> u32 {
        !self.0
    }
}

/// **O quê:** CRC-32 de um bloco único. **Onde:** snapshot e testes.
pub fn crc32(dados: &[u8]) -> u32 {
    Crc32::default().atualizar(dados).fim()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vetor de referência padrão ("123456789" -> 0xCBF43926) e incremental == bloco.
    #[test]
    fn vetor_de_referencia_e_incremental() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        let inc = Crc32::default().atualizar(b"1234").atualizar(b"56789").fim();
        assert_eq!(inc, 0xCBF4_3926);
    }
}
