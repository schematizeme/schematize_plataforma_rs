//! [`Leitura`]: retrato instantâneo do cgroup de uma unidade ou do host.
//!
//! **O quê:** cada métrica é um [`Campo`] (`valor` ou `motivo` da ausência). Falha de leitura,
//! arquivo truncado/hostil ou PSI desligado viram `valor: None` + `motivo`, nunca panic.
//!
//! **Onde:** [`super::observador::Observador`] a cada amostra; o orquestrador para um retrato
//! pontual (ex.: ao fechar a unidade, para alimentar o [`super::historico::Historico`]).

use super::fonte::Fonte;
use super::parse::{
    parse_cpu_stat, parse_io_stat, parse_limite, parse_mem_eventos, parse_mem_stat, parse_meminfo,
    parse_psi, parse_u64, CpuStat, IoResumo, Limite, MemEventos, MemInfo, MemStat, Psi,
};

/// Raiz do cgroup v2 unificado.
pub const CGROUP_RAIZ: &str = "/sys/fs/cgroup";

/// Uma métrica: o valor, ou por que não há.
#[derive(Debug, Clone, PartialEq)]
pub struct Campo<T> {
    /// Valor lido, se houve.
    pub valor: Option<T>,
    /// Motivo da ausência (arquivo inexistente, truncado, não aplicável...).
    pub motivo: Option<String>,
}

impl<T> Campo<T> {
    /// **O quê:** campo com valor. **Onde:** leitura bem-sucedida.
    pub fn ok(v: T) -> Self {
        Campo { valor: Some(v), motivo: None }
    }

    /// **O quê:** campo ausente com `motivo`. **Onde:** leitura que falhou ou não se aplica.
    pub fn falha(motivo: impl Into<String>) -> Self {
        Campo { valor: None, motivo: Some(motivo.into()) }
    }
}

impl<T: Copy> Campo<T> {
    /// **O quê:** o valor, se houver. **Onde:** consumidores que só querem `Option`.
    pub fn v(&self) -> Option<T> {
        self.valor
    }
}

/// Campo pulado pela leitura essencial.
fn nao_lido<T>() -> Campo<T> {
    Campo::falha("não lido (leitura essencial)")
}

/// Lê `caminho` e aplica `p`; qualquer erro (E/S ou parse) vira motivo.
fn ler_campo<T>(f: &dyn Fonte, caminho: &str, p: impl Fn(&str) -> Result<T, String>) -> Campo<T> {
    match f.ler(caminho) {
        Err(e) => Campo::falha(e.to_string()),
        Ok(t) => match p(&t) {
            Ok(v) => Campo::ok(v),
            Err(m) => Campo::falha(format!("{caminho}: {m}")),
        },
    }
}

/// Retrato de memória/recursos de um cgroup ou do host (bytes, salvo nota).
#[derive(Debug, Clone, PartialEq)]
pub struct Leitura {
    /// Diretório do cgroup, ou `"host"`.
    pub origem: String,
    /// `memory.current`.
    pub memoria_atual: Campo<u64>,
    /// `memory.peak` (kernel >= 5.19).
    pub memoria_pico: Campo<u64>,
    /// `memory.max` (aceita `max`).
    pub memoria_max: Campo<Limite>,
    /// `memory.high` (aceita `max`).
    pub memoria_high: Campo<Limite>,
    /// `memory.stat` (anon, file, kernel, slab, shmem).
    pub stat: Campo<MemStat>,
    /// `memory.swap.current`.
    pub swap_atual: Campo<u64>,
    /// `memory.events` (low, high, max, oom, oom_kill).
    pub eventos: Campo<MemEventos>,
    /// `pids.current`.
    pub pids_atual: Campo<u64>,
    /// `cpu.stat` (usage_usec, nr_throttled).
    pub cpu: Campo<CpuStat>,
    /// `io.stat` somado.
    pub io: Campo<IoResumo>,
    /// PSI de memória.
    pub psi_memoria: Campo<Psi>,
    /// PSI de CPU.
    pub psi_cpu: Campo<Psi>,
    /// PSI de I/O.
    pub psi_io: Campo<Psi>,
    /// `/proc/meminfo` (só no host).
    pub meminfo: Campo<MemInfo>,
}

impl Leitura {
    /// **O quê:** leitura COMPLETA do cgroup em `dir` (absoluto, ex. `/sys/fs/cgroup/x.scope`).
    /// **Onde:** retrato pontual; o monitor usa [`Self::de_cgroup_essencial`].
    pub fn de_cgroup(f: &dyn Fonte, dir: &str) -> Leitura {
        let mut l = Self::de_cgroup_essencial(f, dir);
        l.pico_e_extras(f, dir);
        l
    }

    /// **O quê:** só o que o monitor precisa (current, max, high, events, 3 PSI); o resto fica
    /// `falha("não lido")`. **Onde:** [`super::observador::Observador::amostrar`], por alvo.
    pub fn de_cgroup_essencial(f: &dyn Fonte, dir: &str) -> Leitura {
        Leitura {
            origem: dir.to_string(),
            memoria_atual: ler_campo(f, &format!("{dir}/memory.current"), parse_u64),
            memoria_pico: nao_lido(),
            memoria_max: ler_campo(f, &format!("{dir}/memory.max"), parse_limite),
            memoria_high: ler_campo(f, &format!("{dir}/memory.high"), parse_limite),
            stat: nao_lido(),
            swap_atual: nao_lido(),
            eventos: ler_campo(f, &format!("{dir}/memory.events"), parse_mem_eventos),
            pids_atual: nao_lido(),
            cpu: nao_lido(),
            io: nao_lido(),
            psi_memoria: ler_campo(f, &format!("{dir}/memory.pressure"), parse_psi),
            psi_cpu: ler_campo(f, &format!("{dir}/cpu.pressure"), parse_psi),
            psi_io: ler_campo(f, &format!("{dir}/io.pressure"), parse_psi),
            meminfo: Campo::falha("não aplicável a cgroup"),
        }
    }

    /// Completa os campos que a leitura essencial pula.
    fn pico_e_extras(&mut self, f: &dyn Fonte, dir: &str) {
        self.memoria_pico = ler_campo(f, &format!("{dir}/memory.peak"), parse_u64);
        self.stat = ler_campo(f, &format!("{dir}/memory.stat"), parse_mem_stat);
        self.swap_atual = ler_campo(f, &format!("{dir}/memory.swap.current"), parse_u64);
        self.pids_atual = ler_campo(f, &format!("{dir}/pids.current"), parse_u64);
        self.cpu = ler_campo(f, &format!("{dir}/cpu.stat"), parse_cpu_stat);
        self.io = ler_campo(f, &format!("{dir}/io.stat"), parse_io_stat);
    }

    /// **O quê:** retrato do host: `/proc/meminfo`, `/proc/pressure/*` e os arquivos da raiz do
    /// cgroup que existirem (a raiz não tem `memory.current`: vira motivo). **Onde:** decisão
    /// do [`super::governador::Governador`] (MemAvailable, PSI do host).
    pub fn do_host(f: &dyn Fonte) -> Leitura {
        let mut l = Self::de_cgroup(f, CGROUP_RAIZ);
        l.origem = "host".to_string();
        l.meminfo = ler_campo(f, "/proc/meminfo", parse_meminfo);
        l.psi_memoria = ler_campo(f, "/proc/pressure/memory", parse_psi);
        l.psi_cpu = ler_campo(f, "/proc/pressure/cpu", parse_psi);
        l.psi_io = ler_campo(f, "/proc/pressure/io", parse_psi);
        l
    }

    /// **O quê:** todos os motivos de ausência (para log/diagnóstico). **Onde:** o chamador loga
    /// com `trace_id`; nada fica engolido.
    pub fn motivos(&self) -> Vec<&str> {
        [
            self.memoria_atual.motivo.as_deref(),
            self.memoria_pico.motivo.as_deref(),
            self.memoria_max.motivo.as_deref(),
            self.memoria_high.motivo.as_deref(),
            self.stat.motivo.as_deref(),
            self.swap_atual.motivo.as_deref(),
            self.eventos.motivo.as_deref(),
            self.pids_atual.motivo.as_deref(),
            self.cpu.motivo.as_deref(),
            self.io.motivo.as_deref(),
            self.psi_memoria.motivo.as_deref(),
            self.psi_cpu.motivo.as_deref(),
            self.psi_io.motivo.as_deref(),
            self.meminfo.motivo.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}
