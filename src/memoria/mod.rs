//! **memoria** — controle fino de memória e recursos em runtime (std-only, sem dependências).
//!
//! **O quê:** para um orquestrador que roda muitas unidades jauladas (teto duro 500 MiB–2 GiB):
//! - [`Leitura`]: retrato de um cgroup v2 ou do host (current/peak/stat/events/PSI/pids/cpu/io),
//!   com falha de leitura virando `Campo` ausente + motivo, nunca panic;
//! - [`Monitor`]: thread única, anel de capacidade fixa, [`Evento`]s por canal limitado
//!   (`CruzouHigh`, `PressaoPsi`, `CrescimentoRapido` com projeção, `OomKillOcorreu`,
//!   `HostApertado`);
//! - [`Governador`]: política pura (admissão com reserva, estrangular, pedir encolher/parar,
//!   deixar o teto duro agir) com histerese, saída [`Decisao`];
//! - [`Historico`]: picos por tipo de tarefa (texto linha a linha) → recomendação de teto
//!   `p95 x 1,25` clampada em 500–2048 MiB.
//!
//! Tudo lê o kernel por [`Fonte`] (`/proc`, `/sys`), então é testável com cgroupfs simulado
//! ([`FixtureDir`] + [`FonteDir::com_raiz`]). Não depende do módulo `sistema`.
//!
//! **Onde:** o orquestrador cria um [`Monitor`], consome os [`Evento`]s, monta o
//! [`EstadoHost`] e chama [`Governador::decidir`] a cada tick; o ledger alimenta o [`Historico`].
//! Com a feature `jaula`, [`PedidoAdmissao::de_orcamento`] converte um `jaula::Orcamento`.

pub mod anel;
pub mod evento;
pub mod fixture;
pub mod fonte;
pub mod governador;
pub mod historico;
pub mod leitura;
pub mod monitor;
pub mod observador;
pub mod parse;

pub use evento::{Alvo, ConfigMonitor, Evento, Origem, Recurso, INTERVALO_MAX, INTERVALO_MIN};
pub use fixture::FixtureDir;
pub use fonte::{ErroLeitura, Fonte, FonteDir};
pub use governador::{
    Decisao, EstadoHost, Governador, Motivo, PedidoAdmissao, Politica, UnidadeAtiva,
};
pub use historico::{
    percentil95, recomendar_teto_mib, Carga, ErroHistorico, Historico, LinhaRejeitada,
    TETO_MAX_MIB, TETO_MIN_MIB,
};
pub use leitura::{Campo, Leitura, CGROUP_RAIZ};
pub use monitor::{ErroMonitor, Monitor};
pub use observador::{derivada_mib_s, Observador};
pub use parse::{CpuStat, IoResumo, Limite, MemEventos, MemInfo, MemStat, Psi, PsiLinha};
