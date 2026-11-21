//! [`Governador`]: política de admissão/estrangulamento, PURA e determinística.
//!
//! **O quê:** dado o estado do host, as unidades ativas e a fila, [`Governador::decidir`]
//! devolve [`Decisao`]s. Sem I/O, sem relógio (`agora_ms` entra por parâmetro). O único estado
//! é a histerese, explícito no struct. Escada por unidade, sempre do mais suave ao mais duro:
//! 1. admissão com reserva (só entra quem cabe);
//! 2. **estrangular** (reduz paralelismo; não mata ninguém);
//! 3. [`Decisao::PedirEncolher`] (a unidade passou do `high`/projeta estourar, ou é a maior com
//!    o host sem margem): SIGTERM gracioso antes do teto;
//! 4. [`Decisao::PedirParar`] se, passada a graça, o problema persiste;
//! 5. [`Decisao::DeixarTetoAgir`] só se ainda assim ficar a 95% do teto: o cgroup (teto duro)
//!    assume e o OOM fica confinado à unidade.
//!
//! **Admissão com reserva:** `soma(tetos admitidos) + soma(teto - atual das ativas) <=
//! MemAvailable - margem`. As ativas já estão descontadas do MemAvailable pelo que usam; a
//! folga que ainda podem tomar (`teto - atual`) é reservada. Com ativas em `atual = 0` isso é
//! exatamente "soma dos tetos <= MemAvailable - margem". FIFO: para no 1º que não cabe.
//!
//! **Histerese:** entra em aperto com `disponível < margem` ou PSI `some` >= `psi_estrangula`;
//! só sai com `disponível >= 125% da margem` E PSI < `psi_libera`; o paralelismo volta de
//! +1 por decisão (nunca de uma vez). Um pedido por unidade só é rearmado abaixo de 70% do teto.
//!
//! **Onde:** o orquestrador chama a cada tick do [`super::monitor::Monitor`], com o
//! [`super::leitura::Leitura`] do host e das unidades.

use std::collections::BTreeMap;

pub use super::historico::{recomendar_teto_mib, TETO_MAX_MIB, TETO_MIN_MIB};

/// Parâmetros da política.
#[derive(Debug, Clone, PartialEq)]
pub struct Politica {
    /// Margem do host que nunca se compromete (MiB).
    pub margem_mib: u64,
    /// Paralelismo máximo absoluto.
    pub max_paralelo: usize,
    /// Uso (% do teto) a partir do qual a unidade é "acima do high".
    pub high_pct: u64,
    /// Pede para encolher se o teto é atingido em menos que isto (s) no ritmo atual.
    pub horizonte_s: f64,
    /// PSI `some avg10` do host que ativa o estrangulamento (%).
    pub psi_estrangula: f64,
    /// PSI `some avg10` abaixo do qual o estrangulamento pode acabar (%).
    pub psi_libera: f64,
    /// Tempo entre os degraus da escada de uma unidade (ms).
    pub graca_ms: u64,
}

impl Default for Politica {
    /// **O quê:** margem 1 GiB, 8 paralelas, high 85%, horizonte 30 s, PSI 10/4, graça 10 s.
    /// **Onde:** ponto de partida.
    fn default() -> Self {
        Politica {
            margem_mib: 1024,
            max_paralelo: 8,
            high_pct: 85,
            horizonte_s: 30.0,
            psi_estrangula: 10.0,
            psi_libera: 4.0,
            graca_ms: 10_000,
        }
    }
}

/// Estado instantâneo do host.
#[derive(Debug, Clone, PartialEq)]
pub struct EstadoHost {
    /// Relógio monotônico (ms) do chamador.
    pub agora_ms: u64,
    /// `MemTotal` (MiB).
    pub mem_total_mib: u64,
    /// `MemAvailable` (MiB).
    pub mem_disponivel_mib: u64,
    /// PSI de memória `some avg10` do host (%).
    pub psi_some_avg10: f64,
}

/// Unidade em execução.
#[derive(Debug, Clone, PartialEq)]
pub struct UnidadeAtiva {
    /// Identificador estável da unidade.
    pub id: String,
    /// Teto duro (MiB).
    pub teto_mib: u64,
    /// `memory.current` (MiB).
    pub atual_mib: u64,
    /// Derivada de crescimento (MiB/s; <= 0 se estável).
    pub mib_por_s: f64,
}

/// Unidade na fila esperando admissão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PedidoAdmissao {
    /// Identificador da unidade.
    pub id: String,
    /// Teto duro que ela vai ter (MiB).
    pub teto_mib: u64,
}

#[cfg(feature = "jaula")]
impl PedidoAdmissao {
    /// **O quê:** pedido a partir de um [`Orcamento`](crate::jaula::Orcamento) da jaula (usa
    /// `mem_max_mib`). **Onde:** orquestrador que já estimou o orçamento. Só com `jaula`.
    pub fn de_orcamento(id: impl Into<String>, o: &crate::jaula::Orcamento) -> Self {
        PedidoAdmissao { id: id.into(), teto_mib: o.mem_max_mib }
    }
}

/// Por que uma unidade foi acionada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motivo {
    /// Uso passou de `high_pct` do teto.
    AcimaDoHigh,
    /// No ritmo atual atinge o teto dentro do horizonte.
    ProjecaoEstoura,
    /// Host sem margem e esta é a maior consumidora.
    MaiorConsumidora,
    /// Pedido anterior não resolveu dentro da graça.
    PedidoIgnorado,
}

/// Decisão da política.
#[derive(Debug, Clone, PartialEq)]
pub enum Decisao {
    /// Nada a fazer.
    Manter,
    /// Pode iniciar as `quantas` primeiras da fila.
    Admitir {
        /// Quantas da fila (em ordem).
        quantas: usize,
    },
    /// Reduzir o paralelismo total a `paralelismo_max` (sem matar ninguém).
    Estrangular {
        /// Novo limite de unidades paralelas.
        paralelismo_max: usize,
    },
    /// Fim do aperto; o paralelismo sobe gradualmente.
    Liberar,
    /// SIGTERM gracioso pedindo que a unidade encolha.
    PedirEncolher {
        /// Unidade.
        id: String,
        /// Motivo.
        motivo: Motivo,
    },
    /// SIGTERM gracioso pedindo que a unidade termine.
    PedirParar {
        /// Unidade.
        id: String,
        /// Motivo.
        motivo: Motivo,
    },
    /// Nada mais a pedir: o teto duro do cgroup age (OOM confinado).
    DeixarTetoAgir {
        /// Unidade.
        id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Estagio {
    Encolher,
    Parar,
    Teto,
}

/// Política com histerese; ver o módulo.
#[derive(Debug, Clone)]
pub struct Governador {
    politica: Politica,
    estrangulado: bool,
    limite_paralelo: usize,
    ultimo_degrau_ms: u64,
    pedidos: BTreeMap<String, (Estagio, u64)>,
}

impl Governador {
    /// **O quê:** governador na política dada, sem aperto. **Onde:** início do orquestrador.
    pub fn novo(politica: Politica) -> Self {
        let max = politica.max_paralelo.max(1);
        Governador {
            politica,
            estrangulado: false,
            limite_paralelo: max,
            ultimo_degrau_ms: 0,
            pedidos: BTreeMap::new(),
        }
    }

    /// **O quê:** limite de paralelismo vigente. **Onde:** testes/telemetria.
    pub fn limite_paralelo(&self) -> usize {
        self.limite_paralelo
    }

    /// **O quê:** maior teto que o host suporta (`MemTotal - margem`), p/ [`recomendar_teto_mib`].
    /// **Onde:** orquestrador antes de recomendar.
    pub fn max_host_mib(&self, h: &EstadoHost) -> u64 {
        h.mem_total_mib.saturating_sub(self.politica.margem_mib)
    }

    /// **O quê:** decide. Ordem da saída: estrangulamento, pedidos às unidades (por `id`),
    /// admissão; nunca vazia (`[Manter]` se nada a fazer). **Onde:** tick do orquestrador.
    pub fn decidir(
        &mut self,
        h: &EstadoHost,
        ativas: &[UnidadeAtiva],
        fila: &[PedidoAdmissao],
    ) -> Vec<Decisao> {
        let mut out = Vec::new();
        let aperto = self.atualizar_modo(h, ativas.len(), &mut out);
        self.escada_unidades(h, ativas, aperto, &mut out);
        let quantas = self.admissao(h, ativas, fila);
        if quantas > 0 {
            out.push(Decisao::Admitir { quantas });
        }
        if out.is_empty() {
            out.push(Decisao::Manter);
        }
        out
    }

    /// Histerese de aperto/estrangulamento; devolve se o host está em aperto agora.
    fn atualizar_modo(&mut self, h: &EstadoHost, n_ativas: usize, out: &mut Vec<Decisao>) -> bool {
        let p = &self.politica;
        let max = p.max_paralelo.max(1);
        let aperto = h.mem_disponivel_mib < p.margem_mib || h.psi_some_avg10 >= p.psi_estrangula;
        let folga = u128::from(h.mem_disponivel_mib) * 100 >= u128::from(p.margem_mib) * 125;
        let calmo = folga && h.psi_some_avg10 < p.psi_libera;
        if aperto && !self.estrangulado {
            self.estrangulado = true;
            self.limite_paralelo = n_ativas.saturating_sub(1).clamp(1, max);
            self.ultimo_degrau_ms = h.agora_ms;
            out.push(Decisao::Estrangular { paralelismo_max: self.limite_paralelo });
        } else if aperto && self.estrangulado && self.reduzir_mais(h, n_ativas) {
            out.push(Decisao::Estrangular { paralelismo_max: self.limite_paralelo });
        } else if self.estrangulado && calmo {
            self.estrangulado = false;
            out.push(Decisao::Liberar);
        } else if !self.estrangulado {
            self.limite_paralelo = (self.limite_paralelo + 1).min(max);
        }
        aperto
    }

    /// Novo degrau de redução se a graça passou e ainda há unidades acima do limite.
    fn reduzir_mais(&mut self, h: &EstadoHost, n_ativas: usize) -> bool {
        let passou = h.agora_ms.saturating_sub(self.ultimo_degrau_ms) >= self.politica.graca_ms;
        if passou && n_ativas >= self.limite_paralelo && self.limite_paralelo > 1 {
            self.limite_paralelo -= 1;
            self.ultimo_degrau_ms = h.agora_ms;
            return true;
        }
        false
    }

    /// Escada encolher → parar → teto, por unidade, mais a "maior consumidora" no aperto.
    fn escada_unidades(
        &mut self,
        h: &EstadoHost,
        ativas: &[UnidadeAtiva],
        aperto: bool,
        out: &mut Vec<Decisao>,
    ) {
        self.pedidos.retain(|id, _| ativas.iter().any(|u| &u.id == id));
        let mut ordem: Vec<&UnidadeAtiva> = ativas.iter().collect();
        ordem.sort_by(|a, b| a.id.cmp(&b.id));
        for u in &ordem {
            self.avancar_unidade(u, h.agora_ms, out);
        }
        let sem_pedido = ordem.iter().all(|u| !self.pedidos.contains_key(&u.id));
        let sem_margem = h.mem_disponivel_mib < self.politica.margem_mib;
        if aperto && sem_margem && sem_pedido {
            let maior =
                ordem.iter().max_by(|a, b| a.atual_mib.cmp(&b.atual_mib).then(b.id.cmp(&a.id)));
            if let Some(u) = maior {
                self.pedidos.insert(u.id.clone(), (Estagio::Encolher, h.agora_ms));
                out.push(Decisao::PedirEncolher {
                    id: u.id.clone(),
                    motivo: Motivo::MaiorConsumidora,
                });
            }
        }
    }

    /// Motivo pelo qual a unidade precisa de ação agora (`None` = saudável).
    fn motivo(&self, u: &UnidadeAtiva) -> Option<Motivo> {
        if u.atual_mib.saturating_mul(100) >= u.teto_mib.saturating_mul(self.politica.high_pct) {
            return Some(Motivo::AcimaDoHigh);
        }
        let resta = u.teto_mib.saturating_sub(u.atual_mib) as f64;
        (u.mib_por_s > 0.0 && resta / u.mib_por_s <= self.politica.horizonte_s)
            .then_some(Motivo::ProjecaoEstoura)
    }

    /// Um passo da escada para a unidade `u`.
    fn avancar_unidade(&mut self, u: &UnidadeAtiva, agora: u64, out: &mut Vec<Decisao>) {
        let motivo = self.motivo(u);
        let Some(&(est, desde)) = self.pedidos.get(&u.id) else {
            if let Some(m) = motivo {
                self.pedidos.insert(u.id.clone(), (Estagio::Encolher, agora));
                out.push(Decisao::PedirEncolher { id: u.id.clone(), motivo: m });
            }
            return;
        };
        if motivo.is_none() && u.atual_mib.saturating_mul(100) < u.teto_mib.saturating_mul(70) {
            self.pedidos.remove(&u.id);
            return;
        }
        if agora.saturating_sub(desde) < self.politica.graca_ms {
            return;
        }
        match est {
            Estagio::Encolher => {
                self.pedidos.insert(u.id.clone(), (Estagio::Parar, agora));
                out.push(Decisao::PedirParar { id: u.id.clone(), motivo: Motivo::PedidoIgnorado });
            }
            Estagio::Parar if u.atual_mib.saturating_mul(100) >= u.teto_mib.saturating_mul(95) => {
                self.pedidos.insert(u.id.clone(), (Estagio::Teto, agora));
                out.push(Decisao::DeixarTetoAgir { id: u.id.clone() });
            }
            Estagio::Parar | Estagio::Teto => {}
        }
    }

    /// Quantas da fila podem entrar agora (FIFO, com reserva; ver o módulo).
    fn admissao(&self, h: &EstadoHost, ativas: &[UnidadeAtiva], fila: &[PedidoAdmissao]) -> usize {
        if self.estrangulado {
            return 0;
        }
        let reserva: u64 = ativas.iter().map(|u| u.teto_mib.saturating_sub(u.atual_mib)).sum();
        let mut orcamento =
            h.mem_disponivel_mib.saturating_sub(self.politica.margem_mib).saturating_sub(reserva);
        let vagas = self.limite_paralelo.saturating_sub(ativas.len());
        let mut n = 0;
        for p in fila.iter().take(vagas) {
            if p.teto_mib > orcamento {
                break;
            }
            orcamento -= p.teto_mib;
            n += 1;
        }
        n
    }
}
