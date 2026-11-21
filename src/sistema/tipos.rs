//! Modelo de dados do [`Perfil`]: tudo `Option`/[`Lido`] com a fonte e o motivo da ausência.

use crate::sistema::erro::ErroSistema;
use std::fmt::Display;

/// O quê: valor lido de uma fonte, ou o motivo de não ter sido lido.
/// Onde: todo campo de [`Perfil`]; garante "ausente vira None com motivo".
#[derive(Debug, Clone, PartialEq)]
pub struct Lido<T> {
    /// Valor, se a leitura e o parse deram certo.
    pub valor: Option<T>,
    /// Arquivo/comando de onde veio (ou onde se tentou ler).
    pub fonte: String,
    /// Por que `valor` é `None`.
    pub motivo: Option<String>,
}

impl<T> Lido<T> {
    /// O quê: leitura bem-sucedida. Onde: coletores.
    pub fn ok(valor: T, fonte: &str) -> Self {
        Lido { valor: Some(valor), fonte: fonte.to_string(), motivo: None }
    }
    /// O quê: leitura falha com motivo. Onde: coletores.
    pub fn falha(fonte: &str, motivo: impl Display) -> Self {
        Lido { valor: None, fonte: fonte.to_string(), motivo: Some(motivo.to_string()) }
    }
    /// O quê: converte `Result` em `Lido`. Onde: coletores.
    pub fn de(fonte: &str, r: Result<T, ErroSistema>) -> Self {
        match r {
            Ok(v) => Self::ok(v, fonte),
            Err(e) => Self::falha(fonte, e),
        }
    }
    /// O quê: referência ao valor, se houver. Onde: achados/margem.
    pub fn get(&self) -> Option<&T> {
        self.valor.as_ref()
    }
}

/// Limite de cgroup: `max` (sem teto) ou bytes/contagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limite {
    /// Sem teto.
    Max,
    /// Teto numérico.
    Valor(u64),
}

impl Limite {
    /// O quê: teto numérico ou `None` se `max`. Onde: cálculo do efetivo.
    pub fn teto(self) -> Option<u64> {
        match self {
            Limite::Max => None,
            Limite::Valor(v) => Some(v),
        }
    }
}

/// Valores de /proc/meminfo, em KiB.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meminfo {
    /// MemTotal.
    pub total_kib: Option<u64>,
    /// MemAvailable.
    pub disponivel_kib: Option<u64>,
    /// SwapTotal.
    pub swap_total_kib: Option<u64>,
    /// SwapFree.
    pub swap_livre_kib: Option<u64>,
}

/// Dispositivo zram.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Zram {
    /// Nome (zram0).
    pub nome: String,
    /// disksize em bytes.
    pub disksize_bytes: Option<u64>,
    /// Algoritmo ativo (o entre colchetes em comp_algorithm).
    pub algoritmo: Option<String>,
    /// Prioridade em /proc/swaps, se estiver como swap.
    pub prioridade: Option<i32>,
}

/// Linha de /proc/swaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapDev {
    /// Dispositivo/arquivo.
    pub arquivo: String,
    /// Tamanho em KiB.
    pub tamanho_kib: u64,
    /// Usado em KiB.
    pub usado_kib: u64,
    /// Prioridade.
    pub prioridade: i32,
}

/// Parâmetros sysctl vm.* pedidos.
#[derive(Debug, Clone, PartialEq)]
pub struct Vm {
    /// vm.swappiness.
    pub swappiness: Lido<i64>,
    /// vm.overcommit_memory.
    pub overcommit_memory: Lido<i64>,
    /// vm.min_free_kbytes.
    pub min_free_kbytes: Lido<i64>,
    /// vm.page-cluster.
    pub page_cluster: Lido<i64>,
    /// vm.vfs_cache_pressure.
    pub vfs_cache_pressure: Lido<i64>,
    /// vm.watermark_scale_factor.
    pub watermark_scale_factor: Lido<i64>,
    /// vm.oom_kill_allocating_task.
    pub oom_kill_allocating_task: Lido<i64>,
}

/// Linha PSI (some/full).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsiLinha {
    /// Média de 10 s (%).
    pub avg10: f64,
    /// Média de 60 s (%).
    pub avg60: f64,
    /// Média de 300 s (%).
    pub avg300: f64,
    /// Total acumulado em µs.
    pub total_us: u64,
}

/// PSI de um recurso. CPU não tem `full` em kernels antigos.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PsiRecurso {
    /// Linha `some`.
    pub some: Option<PsiLinha>,
    /// Linha `full`.
    pub full: Option<PsiLinha>,
}

/// PSI de memória/cpu/io.
#[derive(Debug, Clone, PartialEq)]
pub struct Psi {
    /// /proc/pressure/memory.
    pub memoria: Lido<PsiRecurso>,
    /// /proc/pressure/cpu.
    pub cpu: Lido<PsiRecurso>,
    /// /proc/pressure/io.
    pub io: Lido<PsiRecurso>,
}

/// Um nível da hierarquia cgroup (do próprio até o topo).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NivelCgroup {
    /// Caminho relativo a /sys/fs/cgroup.
    pub caminho: String,
    /// memory.max.
    pub memory_max: Option<Limite>,
    /// memory.high.
    pub memory_high: Option<Limite>,
    /// memory.swap.max.
    pub swap_max: Option<Limite>,
    /// pids.max.
    pub pids_max: Option<Limite>,
    /// memory.current (bytes).
    pub memory_current: Option<u64>,
    /// Arquivos que não puderam ser lidos, com motivo.
    pub motivos: Vec<String>,
}

/// Contadores de memory.events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventosMem {
    /// low.
    pub low: Option<u64>,
    /// high.
    pub high: Option<u64>,
    /// max.
    pub max: Option<u64>,
    /// oom.
    pub oom: Option<u64>,
    /// oom_kill.
    pub oom_kill: Option<u64>,
}

/// Limites efetivos (mínimo numérico subindo a hierarquia). `None` = sem teto/desconhecido.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Efetivo {
    /// Menor memory.max numérico.
    pub memory_max: Option<u64>,
    /// Menor memory.high numérico.
    pub memory_high: Option<u64>,
    /// Menor memory.swap.max numérico.
    pub swap_max: Option<u64>,
    /// Menor pids.max numérico.
    pub pids_max: Option<u64>,
    /// Menor folga (teto - current) entre níveis com ambos conhecidos.
    pub folga_bytes: Option<u64>,
}

/// user@UID.service: limites e controladores delegados.
#[derive(Debug, Clone, PartialEq)]
pub struct UserSlice {
    /// Caminho relativo.
    pub caminho: String,
    /// cgroup.controllers (delegados ao usuário).
    pub delegados: Lido<Vec<String>>,
    /// Limites do nível.
    pub nivel: NivelCgroup,
}

/// Tudo de cgroup v2.
#[derive(Debug, Clone, PartialEq)]
pub struct Cgroup {
    /// cgroup v2 unificado ativo?
    pub v2: Lido<bool>,
    /// Controladores disponíveis na raiz.
    pub controladores: Lido<Vec<String>>,
    /// Cgroup do próprio processo (/proc/self/cgroup).
    pub proprio: Lido<String>,
    /// Níveis do próprio até o topo.
    pub niveis: Vec<NivelCgroup>,
    /// Limites efetivos.
    pub efetivo: Efetivo,
    /// memory.events do próprio.
    pub eventos: Lido<EventosMem>,
    /// user@UID.service.
    pub user_slice: Option<UserSlice>,
}

/// Estado de um serviço/daemon.
#[derive(Debug, Clone, PartialEq)]
pub struct Servico {
    /// Unidade systemd consultada.
    pub unidade: String,
    /// ActiveState (via systemctl).
    pub active_state: Lido<String>,
    /// SubState (via systemctl).
    pub sub_state: Option<String>,
    /// Processo com este comm encontrado em /proc?
    pub processo: Lido<bool>,
}

impl Servico {
    /// O quê: ativo se a unit está `active` ou o processo existe. Onde: achados.
    pub fn ativo(&self) -> bool {
        self.active_state.get().map(|s| s == "active").unwrap_or(false)
            || self.processo.get().copied().unwrap_or(false)
    }
}

/// Config efetiva do systemd-oomd (oomd.conf + drop-ins), como texto.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OomdConf {
    /// SwapUsedLimit.
    pub swap_used_limit: Option<String>,
    /// DefaultMemoryPressureLimit.
    pub pressao_limite: Option<String>,
    /// DefaultMemoryPressureDurationSec.
    pub pressao_duracao: Option<String>,
}

/// ManagedOOM* do user.slice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManagedOom {
    /// ManagedOOMMemoryPressure.
    pub memoria: Option<String>,
    /// ManagedOOMSwap.
    pub swap: Option<String>,
}

/// DefaultMemoryAccounting/DefaultTasksAccounting do systemd.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Accounting {
    /// DefaultMemoryAccounting.
    pub memoria: Option<String>,
    /// DefaultTasksAccounting.
    pub tarefas: Option<String>,
}

/// Daemons relevantes de OOM/recursos.
#[derive(Debug, Clone, PartialEq)]
pub struct Daemons {
    /// systemd-oomd.
    pub oomd: Servico,
    /// earlyoom.
    pub earlyoom: Servico,
    /// nohang.
    pub nohang: Servico,
    /// ananicy-cpp.
    pub ananicy: Servico,
    /// power-profiles-daemon.
    pub power_profiles: Servico,
    /// Config do oomd.
    pub oomd_conf: Lido<OomdConf>,
    /// ManagedOOM do user.slice.
    pub managed_oom_user: Lido<ManagedOom>,
    /// Accounting padrão do systemd.
    pub accounting: Lido<Accounting>,
}

/// Kernel e distro.
#[derive(Debug, Clone, PartialEq)]
pub struct Kernel {
    /// /proc/version.
    pub versao: Lido<String>,
    /// /proc/sys/kernel/osrelease.
    pub osrelease: Lido<String>,
    /// os-release ID.
    pub distro_id: Lido<String>,
    /// os-release ID_LIKE.
    pub id_like: Vec<String>,
    /// ID ou ID_LIKE contém cachyos.
    pub cachyos: bool,
    /// CPUs lógicas.
    pub cpus: Option<usize>,
    /// UID real do processo.
    pub uid: Lido<u32>,
}

/// oom_score / oom_score_adj do próprio processo.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessoOom {
    /// /proc/self/oom_score.
    pub oom_score: Lido<i64>,
    /// /proc/self/oom_score_adj.
    pub oom_score_adj: Lido<i64>,
}

/// Memória do host.
#[derive(Debug, Clone, PartialEq)]
pub struct Memoria {
    /// /proc/meminfo.
    pub meminfo: Lido<Meminfo>,
    /// Dispositivos zram.
    pub zram: Lido<Vec<Zram>>,
    /// /proc/swaps.
    pub swaps: Lido<Vec<SwapDev>>,
    /// sysctl vm.*.
    pub vm: Vm,
    /// PSI.
    pub psi: Psi,
}

/// Gravidade de um achado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Gravidade {
    /// Informativo (normal no host).
    Info,
    /// Merece atenção.
    Atencao,
    /// Risco real de perda/morte de processo.
    Risco,
}

/// Achado derivado do perfil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Achado {
    /// Gravidade.
    pub gravidade: Gravidade,
    /// Código estável (SYS-...).
    pub codigo: String,
    /// Mensagem em pt.
    pub mensagem: String,
    /// Sugestão em pt.
    pub sugestao: String,
    /// Mensagem em en.
    pub mensagem_en: String,
    /// Sugestão em en.
    pub sugestao_en: String,
}

/// Perfil completo do host.
#[derive(Debug, Clone, PartialEq)]
pub struct Perfil {
    /// cgroup v2.
    pub cgroup: Cgroup,
    /// Memória/swap/zram/vm/PSI.
    pub memoria: Memoria,
    /// Daemons.
    pub daemons: Daemons,
    /// Kernel/distro/CPUs.
    pub kernel: Kernel,
    /// OOM do processo.
    pub processo: ProcessoOom,
    /// Achados derivados.
    pub achados: Vec<Achado>,
}

/// Idioma do resumo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idioma {
    /// Português.
    Pt,
    /// English.
    En,
}

/// Margem de memória que o orquestrador pode reservar, em MiB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MargemMemoria {
    /// MiB reserváveis sem tocar o piso crítico nem o teto de cgroup.
    pub reservavel_mib: u64,
    /// MemAvailable do host em MiB (0 se desconhecido).
    pub disponivel_host_mib: u64,
    /// Piso crítico preservado, em MiB.
    pub piso_critico_mib: u64,
    /// Folga sob o teto de cgroup efetivo, se houver teto.
    pub folga_cgroup_mib: Option<u64>,
    /// Quem limitou o resultado.
    pub limitado_por: LimitadoPor,
}

/// O que determinou a margem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitadoPor {
    /// MemAvailable menos o piso crítico.
    Host,
    /// Teto de cgroup efetivo.
    Cgroup,
    /// Sem dados de memória; margem 0 por segurança.
    Desconhecido,
}
