//! [`Historico`] de picos de memória por tipo de tarefa + recomendação de teto.
//!
//! **O quê:** guarda os últimos [`MAX_PICOS`] picos (MiB) por tipo; serializa linha a linha
//! (`v1|<tipo>|<mib>`, sem serde) e relê tolerando linhas hostis (devolvidas em
//! [`Carga::rejeitadas`], nunca engolidas). [`recomendar_teto_mib`] = p95 x 1,25, clamp 500–2048,
//! nunca acima do que o host suporta.
//!
//! **Onde:** o ledger alimenta com o pico real de cada execução ([`Historico::registrar`]) e
//! o orquestrador pede a recomendação ao estimar a próxima ([`Historico::recomendar`]).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Piso do teto recomendado (MiB); espelha `jaula::MEM_MIN_MIB`.
pub const TETO_MIN_MIB: u64 = 500;
/// Teto máximo recomendado (MiB); espelha `jaula::MEM_MAX_MIB`.
pub const TETO_MAX_MIB: u64 = 2048;
/// Picos guardados por tipo (os mais recentes).
pub const MAX_PICOS: usize = 64;

/// Erro ao registrar um pico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroHistorico {
    /// Tipo vazio, com `|`, quebra de linha ou caractere de controle.
    TipoInvalido(String),
}

impl fmt::Display for ErroHistorico {
    /// **O quê:** mensagem acionável. **Onde:** log do ledger.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroHistorico::TipoInvalido(t) => write!(
                f,
                "tipo de tarefa inválido {t:?}: use texto não vazio sem '|' nem quebra de linha"
            ),
        }
    }
}

impl std::error::Error for ErroHistorico {}

/// Uma linha que não pôde ser lida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinhaRejeitada {
    /// Número da linha (base 1).
    pub numero: usize,
    /// Por que foi rejeitada.
    pub motivo: String,
}

/// Resultado de [`Historico::de_linhas`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carga {
    /// Histórico com as linhas válidas.
    pub historico: Historico,
    /// Linhas rejeitadas, com motivo.
    pub rejeitadas: Vec<LinhaRejeitada>,
}

/// Picos (MiB) recentes por tipo de tarefa.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Historico {
    por_tipo: BTreeMap<String, VecDeque<u64>>,
}

/// Valida o nome do tipo (ver [`ErroHistorico::TipoInvalido`]).
fn tipo_valido(t: &str) -> Result<&str, ErroHistorico> {
    let t = t.trim();
    if t.is_empty() || t.chars().any(|c| c == '|' || c.is_control()) {
        return Err(ErroHistorico::TipoInvalido(t.chars().take(32).collect()));
    }
    Ok(t)
}

impl Historico {
    /// **O quê:** histórico vazio. **Onde:** início do ledger.
    pub fn novo() -> Self {
        Self::default()
    }

    /// **O quê:** registra o pico (MiB) de uma execução do `tipo`; mantém só os últimos
    /// [`MAX_PICOS`]. **Onde:** ledger, ao fechar cada execução.
    pub fn registrar(&mut self, tipo: &str, pico_mib: u64) -> Result<(), ErroHistorico> {
        let fila = self.por_tipo.entry(tipo_valido(tipo)?.to_string()).or_default();
        if fila.len() == MAX_PICOS {
            fila.pop_front();
        }
        fila.push_back(pico_mib);
        Ok(())
    }

    /// **O quê:** picos do tipo, do mais antigo ao mais novo. **Onde:** estatística/testes.
    pub fn picos(&self, tipo: &str) -> Vec<u64> {
        self.por_tipo.get(tipo.trim()).map(|f| f.iter().copied().collect()).unwrap_or_default()
    }

    /// **O quê:** tipos conhecidos (ordem alfabética). **Onde:** relatórios.
    pub fn tipos(&self) -> Vec<&str> {
        self.por_tipo.keys().map(String::as_str).collect()
    }

    /// **O quê:** teto recomendado para `tipo` (`None` sem histórico). **Onde:** orquestrador
    /// antes de estimar o [`Orcamento`](crate::jaula) da próxima execução.
    pub fn recomendar(&self, tipo: &str, max_host_mib: u64) -> Option<u64> {
        recomendar_teto_mib(&self.picos(tipo), max_host_mib)
    }

    /// **O quê:** a linha `v1|<tipo>|<mib>` para anexar ao ledger. **Onde:** append-only.
    pub fn linha(tipo: &str, pico_mib: u64) -> Result<String, ErroHistorico> {
        Ok(format!("v1|{}|{pico_mib}", tipo_valido(tipo)?))
    }

    /// **O quê:** serializa tudo (uma linha por pico, ordem estável). **Onde:** snapshot.
    pub fn serializar(&self) -> String {
        let mut s = String::new();
        for (tipo, fila) in &self.por_tipo {
            for p in fila {
                s.push_str(&format!("v1|{tipo}|{p}\n"));
            }
        }
        s
    }

    /// **O quê:** relê o texto; vazias e `#` são ignoradas de propósito, o resto inválido vai
    /// para [`Carga::rejeitadas`]. **Onde:** boot do orquestrador, lendo o ledger.
    pub fn de_linhas(texto: &str) -> Carga {
        let mut h = Historico::novo();
        let mut rejeitadas = Vec::new();
        for (i, l) in texto.lines().enumerate() {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Err(motivo) = h.aplicar_linha(l) {
                rejeitadas.push(LinhaRejeitada { numero: i + 1, motivo });
            }
        }
        Carga { historico: h, rejeitadas }
    }

    /// Interpreta uma linha `v1|tipo|mib` e registra.
    fn aplicar_linha(&mut self, l: &str) -> Result<(), String> {
        let mut it = l.split('|');
        match (it.next(), it.next(), it.next(), it.next()) {
            (Some("v1"), Some(tipo), Some(mib), None) => {
                let mib: u64 = mib.trim().parse().map_err(|_| format!("MiB inválido {mib:?}"))?;
                self.registrar(tipo, mib).map_err(|e| e.to_string())
            }
            (Some(v), ..) if v != "v1" => Err(format!("versão desconhecida {v:?}")),
            _ => Err("esperado v1|<tipo>|<mib>".to_string()),
        }
    }
}

/// **O quê:** percentil 95 por "nearest rank" (`ceil(0,95 n)`-ésimo menor); `None` se vazio.
/// **Onde:** [`recomendar_teto_mib`].
pub fn percentil95(picos: &[u64]) -> Option<u64> {
    if picos.is_empty() {
        return None;
    }
    let mut v = picos.to_vec();
    v.sort_unstable();
    let rank = (v.len() * 95).div_ceil(100).max(1);
    v.get(rank - 1).copied()
}

/// **O quê:** `p95 x 1,25` (arredonda para cima), clamp [`TETO_MIN_MIB`]–[`TETO_MAX_MIB`] e
/// depois `min(max_host_mib)`: com host minúsculo o resultado pode ficar abaixo do piso — o
/// host manda. `None` sem picos. **Onde:** [`Historico::recomendar`].
pub fn recomendar_teto_mib(picos: &[u64], max_host_mib: u64) -> Option<u64> {
    let p95 = percentil95(picos)?;
    let folga = p95.saturating_mul(125).div_ceil(100);
    Some(folga.clamp(TETO_MIN_MIB, TETO_MAX_MIB).min(max_host_mib))
}
