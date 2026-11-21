//! Orçamento de recursos de uma execução jaulada: classes, estimativa e clamp.
//!
//! **O quê:** [`Orcamento`] (teto duro de memória/pids/cpu/tempo), [`Classe`] (512/1024/2048
//! MiB), [`TipoExecucao`] e a estimativa PURA [`estimar`].
//!
//! **Onde:** quem despacha um subagent/código chama [`estimar`] (ou [`Orcamento::novo`]) e passa
//! o resultado a [`super::executar`].

use std::fmt;
use std::time::Duration;

/// Piso do teto de memória (MiB): abaixo disso quase nada útil roda.
pub const MEM_MIN_MIB: u64 = 500;
/// Teto padrão máximo (MiB): acima disso só com `override_adr`.
pub const MEM_MAX_MIB: u64 = 2048;
/// Memória média de um job de compilação (MiB), base de [`jobs_sugeridos`].
const MIB_POR_JOB: u64 = 700;

/// Classe de teto de memória.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classe {
    /// 512 MiB.
    Leve,
    /// 1024 MiB.
    Medio,
    /// 2048 MiB.
    Pesado,
}

impl Classe {
    /// **O quê:** o teto em MiB da classe.
    /// **Onde:** [`estimar`] e chamadores que pedem uma classe direto.
    pub fn mem_mib(self) -> u64 {
        match self {
            Classe::Leve => 512,
            Classe::Medio => 1024,
            Classe::Pesado => 2048,
        }
    }
}

/// O que vai rodar na jaula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoExecucao {
    /// Subagent LLM.
    AgenteLlm,
    /// Script avulso.
    Script,
    /// Ferramenta (CLI) auxiliar.
    Tool,
    /// Suíte de testes.
    Teste,
    /// Compilação/build.
    Build,
    /// Navegador (headless ou não).
    Navegador,
}

/// Erro de construção de [`Orcamento`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroOrcamento {
    /// Pediu acima de [`MEM_MAX_MIB`] sem `override_adr`.
    AcimaDoTeto {
        /// Valor pedido (MiB).
        pedido_mib: u64,
    },
    /// `override_adr` presente mas vazio.
    AdrVazio,
}

impl fmt::Display for ErroOrcamento {
    /// **O quê:** mensagem acionável e sem culpa. **Onde:** logs de quem constrói o orçamento.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroOrcamento::AcimaDoTeto { pedido_mib } => write!(
                f,
                "{pedido_mib} MiB passa do teto de {MEM_MAX_MIB} MiB; para precisar disso, \
                 registre um ADR e passe o seu id em override_adr"
            ),
            ErroOrcamento::AdrVazio => write!(f, "override_adr está vazio; informe o id do ADR"),
        }
    }
}

impl std::error::Error for ErroOrcamento {}

/// Teto duro de recursos de UMA execução.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orcamento {
    /// Teto duro de memória (MiB) — estourou, morre só esta execução.
    pub mem_max_mib: u64,
    /// Limiar de pressão (MiB): 85% do máximo; o kernel faz throttle antes de matar.
    pub mem_high_mib: u64,
    /// Teto de swap (MiB); sempre 0 por padrão (swap esconde o estouro e trava a máquina).
    pub swap_max_mib: u64,
    /// Máximo de tarefas (processos/threads) na jaula.
    pub pids_max: u64,
    /// Cota de CPU em % de um núcleo (100 = 1 núcleo); `None` = sem cota.
    pub cpu_quota_pct: Option<u32>,
    /// Tempo máximo de parede; `None` = sem limite.
    pub timeout: Option<Duration>,
    /// `OOMScoreAdjust` do jaulado: maior que o do pai, para o kernel preferi-lo.
    pub oom_score_adj: i32,
    /// Id do ADR que autoriza `mem_max_mib` > [`MEM_MAX_MIB`].
    pub override_adr: Option<String>,
}

impl Orcamento {
    /// **O quê:** constrói o orçamento: clampa `mem_max_mib` em `[500, 2048]` por baixo; acima de
    /// 2048 só com `override_adr` (senão `Err(AcimaDoTeto)`); `mem_high` = 85% do máximo.
    /// **Onde:** chamadores que querem um teto específico; o caminho comum é [`estimar`].
    pub fn novo(mem_max_mib: u64, override_adr: Option<String>) -> Result<Self, ErroOrcamento> {
        if matches!(&override_adr, Some(a) if a.trim().is_empty()) {
            return Err(ErroOrcamento::AdrVazio);
        }
        if mem_max_mib > MEM_MAX_MIB && override_adr.is_none() {
            return Err(ErroOrcamento::AcimaDoTeto { pedido_mib: mem_max_mib });
        }
        let max = mem_max_mib.max(MEM_MIN_MIB);
        Ok(Orcamento {
            mem_max_mib: max,
            mem_high_mib: max * 85 / 100,
            swap_max_mib: 0,
            pids_max: 512,
            cpu_quota_pct: None,
            timeout: None,
            oom_score_adj: 800,
            override_adr,
        })
    }

    /// **O quê:** orçamento de uma [`Classe`] (sempre dentro do intervalo, nunca falha).
    /// **Onde:** [`estimar`].
    pub fn da_classe(classe: Classe) -> Self {
        let max = classe.mem_mib();
        Orcamento {
            mem_max_mib: max,
            mem_high_mib: max * 85 / 100,
            swap_max_mib: 0,
            pids_max: 512,
            cpu_quota_pct: None,
            timeout: None,
            oom_score_adj: 800,
            override_adr: None,
        }
    }
}

/// **O quê:** estimativa PURA do orçamento por tipo: AgenteLlm→Medio, Script/Tool→Leve,
/// Teste→Medio, Build→Pesado, Navegador→Pesado.
/// **Onde:** ponto de entrada de quem despacha uma execução.
pub fn estimar(tipo: TipoExecucao) -> Orcamento {
    let classe = match tipo {
        TipoExecucao::AgenteLlm | TipoExecucao::Teste => Classe::Medio,
        TipoExecucao::Script | TipoExecucao::Tool => Classe::Leve,
        TipoExecucao::Build | TipoExecucao::Navegador => Classe::Pesado,
    };
    Orcamento::da_classe(classe)
}

/// **O quê:** jobs paralelos sugeridos para um build dentro do teto (`teto / 700 MiB`, mín. 1).
/// **Onde:** `cargo build -j N` / `make -jN` dentro da jaula.
pub fn jobs_sugeridos(o: &Orcamento) -> u64 {
    (o.mem_max_mib / MIB_POR_JOB).max(1)
}
