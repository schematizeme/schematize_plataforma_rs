//! Anel de capacidade fixa: o monitor guarda só as últimas N amostras (memória constante).
//!
//! **Onde:** [`super::observador`] (histórico curto de `memory.current` por alvo).

/// Buffer circular `Copy` de capacidade fixa (alocada uma vez, nunca cresce).
#[derive(Debug, Clone)]
pub struct Anel<T: Copy> {
    buf: Vec<T>,
    cap: usize,
    inicio: usize,
}

impl<T: Copy> Anel<T> {
    /// **O quê:** anel com `cap` (mínimo 1) posições reservadas. **Onde:** criação do estado do alvo.
    pub fn novo(cap: usize) -> Self {
        let cap = cap.max(1);
        Anel { buf: Vec::with_capacity(cap), cap, inicio: 0 }
    }

    /// **O quê:** insere; cheio, sobrescreve o mais antigo. **Onde:** cada amostra.
    pub fn push(&mut self, v: T) {
        if self.buf.len() < self.cap {
            self.buf.push(v);
        } else {
            self.buf[self.inicio] = v;
            self.inicio = (self.inicio + 1) % self.cap;
        }
    }

    /// **O quê:** quantidade guardada. **Onde:** testes e cálculo de derivada.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// **O quê:** `true` se vazio. **Onde:** par de [`Self::len`].
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// **O quê:** o elemento `i` contando do mais antigo (0) ao mais novo. **Onde:** derivada.
    pub fn get(&self, i: usize) -> Option<T> {
        if i >= self.buf.len() {
            return None;
        }
        self.buf.get((self.inicio + i) % self.buf.len()).copied()
    }

    /// **O quê:** o mais novo. **Onde:** valor corrente da derivada.
    pub fn ultimo(&self) -> Option<T> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }

    /// **O quê:** descarta tudo, mantendo a capacidade. **Onde:** contador/medida que regrediu.
    pub fn limpar(&mut self) {
        self.buf.clear();
        self.inicio = 0;
    }
}
