//! Tipos do monitor: alvo observado, configuração e [`Evento`] emitido.
//!
//! **Onde:** [`super::observador`] produz; [`super::monitor::Monitor`] entrega por canal
//! limitado; o orquestrador/[`super::governador::Governador`] consome.

use std::time::Duration;

/// Intervalo mínimo do monitor.
pub const INTERVALO_MIN: Duration = Duration::from_millis(250);
/// Intervalo máximo do monitor.
pub const INTERVALO_MAX: Duration = Duration::from_secs(5);

/// De onde sai o cgroup observado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origem {
    /// Caminho do cgroup relativo à raiz (`/user.slice/x.scope`).
    Cgroup(String),
    /// PID cujo cgroup é resolvido por `/proc/<pid>/cgroup` a cada amostra.
    Pid(u32),
}

/// Um alvo nomeado do monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alvo {
    /// Nome que aparece nos eventos (ex.: id da unidade).
    pub nome: String,
    /// Como achar o cgroup.
    pub origem: Origem,
}

impl Alvo {
    /// **O quê:** alvo por caminho de cgroup. **Onde:** orquestrador que conhece a unidade.
    pub fn cgroup(nome: impl Into<String>, rel: impl Into<String>) -> Self {
        Alvo { nome: nome.into(), origem: Origem::Cgroup(rel.into()) }
    }

    /// **O quê:** alvo por PID. **Onde:** `systemd-run --scope` (o PID já nasce no scope).
    pub fn pid(nome: impl Into<String>, pid: u32) -> Self {
        Alvo { nome: nome.into(), origem: Origem::Pid(pid) }
    }
}

/// Recurso do PSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recurso {
    /// `memory.pressure`.
    Memoria,
    /// `cpu.pressure`.
    Cpu,
    /// `io.pressure`.
    Io,
}

/// Parâmetros do monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigMonitor {
    /// Intervalo entre amostras (clampado em 250 ms – 5 s).
    pub intervalo: Duration,
    /// Amostras guardadas por alvo (clampado em 4–256).
    pub capacidade_anel: usize,
    /// Capacidade do canal de eventos (mínimo 1); cheio descarta e conta.
    pub capacidade_canal: usize,
    /// `HostApertado` quando MemAvailable fica abaixo disto (MiB).
    pub margem_host_mib: u64,
    /// `PressaoPsi` quando `full avg10` passa disto (%).
    pub psi_full_limiar: f64,
    /// `CrescimentoRapido` quando a derivada passa disto (MiB/s).
    pub crescimento_mib_s: f64,
}

impl Default for ConfigMonitor {
    /// **O quê:** 1 s, anel 32, canal 256, margem 1 GiB, PSI 10%, 50 MiB/s. **Onde:** ponto de partida.
    fn default() -> Self {
        ConfigMonitor {
            intervalo: Duration::from_secs(1),
            capacidade_anel: 32,
            capacidade_canal: 256,
            margem_host_mib: 1024,
            psi_full_limiar: 10.0,
            crescimento_mib_s: 50.0,
        }
    }
}

impl ConfigMonitor {
    /// **O quê:** cópia com os limites aplicados (intervalo, anel, canal, limiares finitos >0).
    /// **Onde:** [`super::observador::Observador::novo`] e [`super::monitor::Monitor::iniciar`].
    pub fn normalizada(&self) -> ConfigMonitor {
        let pos = |v: f64, padrao: f64| if v.is_finite() && v > 0.0 { v } else { padrao };
        let d = ConfigMonitor::default();
        ConfigMonitor {
            intervalo: self.intervalo.clamp(INTERVALO_MIN, INTERVALO_MAX),
            capacidade_anel: self.capacidade_anel.clamp(4, 256),
            capacidade_canal: self.capacidade_canal.max(1),
            margem_host_mib: self.margem_host_mib,
            psi_full_limiar: pos(self.psi_full_limiar, d.psi_full_limiar),
            crescimento_mib_s: pos(self.crescimento_mib_s, d.crescimento_mib_s),
        }
    }
}

/// O que o monitor avisa. Cada evento é disparado na BORDA (uma vez por episódio, com
/// histerese para rearmar), não a cada amostra.
#[derive(Debug, Clone, PartialEq)]
pub enum Evento {
    /// Passou de `memory.high` (uso >= high, ou `memory.events high` subiu).
    CruzouHigh {
        /// Nome do alvo.
        alvo: String,
        /// `memory.current`.
        atual_bytes: u64,
        /// `memory.high` finito, se houver.
        high_bytes: Option<u64>,
        /// Aumento do contador `high` desde a amostra anterior.
        delta_events_high: u64,
    },
    /// `full avg10` do PSI acima do limiar.
    PressaoPsi {
        /// Nome do alvo.
        alvo: String,
        /// Recurso.
        recurso: Recurso,
        /// `full avg10` (%).
        full_avg10: f64,
        /// Limiar configurado (%).
        limiar: f64,
    },
    /// Derivada de `memory.current` acima do limiar, com projeção até o teto.
    CrescimentoRapido {
        /// Nome do alvo.
        alvo: String,
        /// Crescimento (MiB/s).
        mib_por_s: f64,
        /// `memory.current`.
        atual_bytes: u64,
        /// `memory.max` finito, se houver.
        teto_bytes: Option<u64>,
        /// Segundos até o teto no ritmo atual (`None` sem teto).
        segundos_ate_teto: Option<f64>,
    },
    /// `oom_kill` subiu (o teto duro agiu).
    OomKillOcorreu {
        /// Nome do alvo.
        alvo: String,
        /// Aumento desde a amostra anterior.
        delta: u64,
        /// Valor total do contador.
        total: u64,
    },
    /// `MemAvailable` do host abaixo da margem.
    HostApertado {
        /// `MemAvailable` (bytes).
        disponivel_bytes: u64,
        /// Margem configurada (bytes).
        margem_bytes: u64,
    },
    /// Leitura impossível (processo morreu, cgroup sumiu...). Emitido quando o motivo MUDA.
    FalhaLeitura {
        /// Nome do alvo (ou `host`).
        alvo: String,
        /// Motivo.
        motivo: String,
    },
}
