//! **jaula** — teto duro de memória por execução, descentralizado.
//!
//! **O quê:** cada execução (subagent, script, build, teste, navegador) roda numa jaula própria
//! com um [`Orcamento`] (estimado em 500 MiB a 2 GiB por [`estimar`]). Estourou o teto → morre
//! SÓ aquela execução e o chamador recebe [`Veredito::EstourouMemoria`] como VALOR de retorno.
//!
//! **Por quê (OOM confinado ao cgroup):** sem jaula, o OOM do kernel (ou do cgroup pai) escolhe
//! uma vítima entre todos os processos da operação — e leva o orquestrador junto. Com
//! `systemd-run --user --scope -p MemoryMax=…`, a execução vive no próprio cgroup v2: o limite
//! vale ali, o OOM é acionado ali e a vítima só pode ser de dentro dali. `OOMPolicy=kill` liga
//! `memory.oom.group`, então o kernel mata o grupo inteiro da jaula (sem sobras zumbis) e nada
//! fora dele. `MemorySwapMax=0` impede o swap de mascarar o estouro. O `OOMScoreAdjust` do
//! jaulado (800 por padrão) é MAIOR que o do pai (0), então, mesmo numa pressão global, o
//! kernel prefere o jaulado. `MemoryHigh` (85%) faz throttle antes do corte.
//!
//! **Onde:** orquestradores de agentes. Uso típico:
//! `executar(&estimar(TipoExecucao::AgenteLlm), &argv, "agente-42")`.
//!
//! **Fallback e limites:** sem systemd/cgroup delegado, [`Backend::Prlimit`] limita o espaço de
//! endereços por processo (o estouro costuma virar `Falhou`, sem prova de OOM). Sem nenhum dos
//! dois, [`executar`] devolve [`Veredito::SemBackend`] e NÃO roda: nunca executa sem teto.
//! Só Linux/Unix. Std-only, zero dependências.

pub mod backend;
pub mod comando;
pub mod executar;
pub mod orcamento;
pub mod veredito;

pub use backend::{detectar_backend, Backend};
pub use comando::{comando_jaulado, comando_prlimit, sanitizar_unidade};
pub use executar::executar;
pub use orcamento::{
    estimar, jobs_sugeridos, Classe, ErroOrcamento, Orcamento, TipoExecucao, MEM_MAX_MIB,
    MEM_MIN_MIB,
};
pub use veredito::Veredito;

#[cfg(test)]
mod testes;
