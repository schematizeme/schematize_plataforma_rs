//! Gerador pseudo-aleatório próprio (xorshift64*), determinístico por seed.
//! Serve a testes property-style e ao jitter do vigia. NÃO é criptográfico.

/// **O quê:** PRNG determinístico.
///
/// **Onde:** testes (seed registrada na falha) e jitter do backoff do vigia.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// **O quê:** cria o gerador; seed 0 vira uma constante não nula. **Onde:** início de teste.
    pub fn novo(seed: u64) -> Self {
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    /// **O quê:** próximo `u64`. **Onde:** base dos demais métodos.
    pub fn proximo(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// **O quê:** inteiro em `0..n` (`n == 0` devolve 0). **Onde:** escolhas aleatórias.
    pub fn ate(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.proximo() % n
        }
    }

    /// **O quê:** `len` bytes pseudo-aleatórios. **Onde:** payloads e arquivos-lixo de teste.
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.proximo() >> 24) as u8).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mesma seed, mesma sequência; seed 0 não trava em zero.
    #[test]
    fn determinismo() {
        let (mut a, mut b) = (Rng::novo(7), Rng::novo(7));
        assert_eq!(a.bytes(32), b.bytes(32));
        assert_ne!(Rng::novo(0).proximo(), 0);
        assert_eq!(Rng::novo(1).ate(0), 0);
    }
}
