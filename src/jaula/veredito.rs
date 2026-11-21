//! Veredito de uma execução e o parsing PURO das evidências de OOM.
//!
//! **O quê:** [`Veredito`], parsers de `memory.events`/`systemctl show`/`/proc/<pid>/cgroup` e a
//! classificação final [`classificar`].
//! **Onde:** [`super::executar`] coleta o texto; aqui tudo é testável com fixtures.

/// Resultado de [`super::executar`]. Morte por OOM é um VALOR, nunca pânico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Veredito {
    /// Terminou com este código (0 = sucesso).
    Concluiu(i32),
    /// Terminou com falha (código != 0, ou 128+sinal; 127 = nem chegou a executar).
    Falhou(i32),
    /// O kernel matou a execução por estourar o teto; só ela morreu.
    EstourouMemoria {
        /// Maior uso observado (MiB), se foi possível medir.
        pico_mib: Option<u64>,
        /// Teto que valia (MiB).
        teto_mib: u64,
    },
    /// Passou do tempo; a unidade foi morta.
    Timeout,
    /// Sem como impor teto: nada foi executado. Traz o motivo.
    SemBackend(String),
}

/// Evidências coletadas durante/depois da execução.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidencia {
    /// Maior `oom_kill` visto em `memory.events`.
    pub oom_kill: u64,
    /// `systemctl show -p Result` disse `oom-kill`.
    pub result_oom: bool,
    /// Maior uso observado (bytes).
    pub pico_bytes: Option<u64>,
    /// Última leitura de memória anônima (bytes), de `memory.stat`.
    pub anon_bytes: Option<u64>,
    /// Encalhou no `MemoryHigh`: memória anônima (irrecuperável sem swap) acima do high por
    /// tempo demais; o kernel só faz throttle e o OOM nunca chega. A unidade foi morta.
    pub encalhou: bool,
}

/// **O quê:** valor de `oom_kill` num texto de `memory.events` (`None` se o campo falta).
/// **Onde:** [`super::executar`] a cada amostra do cgroup.
pub fn parse_oom_kill(memory_events: &str) -> Option<u64> {
    memory_events.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        (it.next()? == "oom_kill").then(|| it.next()?.parse().ok()).flatten()
    })
}

/// **O quê:** `true` se a saída de `systemctl show -p Result` é `Result=oom-kill`.
/// **Onde:** [`super::executar`], quando houve SIGKILL sem prova em `memory.events`.
pub fn parse_result_oom(show: &str) -> bool {
    show.lines().any(|l| l.trim() == "Result=oom-kill")
}

/// **O quê:** caminho do cgroup v2 (`0::/...`) de um `/proc/<pid>/cgroup`.
/// **Onde:** localizar o diretório do scope em `/sys/fs/cgroup`.
pub fn parse_cgroup_path(proc_cgroup: &str) -> Option<String> {
    proc_cgroup.lines().find_map(|l| l.strip_prefix("0::")).map(|p| p.trim().to_string())
}

/// **O quê:** campo `anon` (bytes) de um texto de `memory.stat`.
/// **Onde:** detecção de encalhe no `MemoryHigh` em [`super::executar`].
pub fn parse_anon(memory_stat: &str) -> Option<u64> {
    memory_stat.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        (it.next()? == "anon").then(|| it.next()?.parse().ok()).flatten()
    })
}

/// **O quê:** converte o texto de `memory.peak`/`memory.current` em bytes.
/// **Onde:** amostragem de pico em [`super::executar`].
pub fn parse_bytes(texto: &str) -> Option<u64> {
    texto.trim().parse().ok()
}

/// **O quê:** classificação PURA: `codigo` (exit) e `sinal` do processo + evidências → veredito.
/// OOM se há prova (`oom_kill>0`/Result) e a execução falhou, ou se foi SIGKILL com pico >= 90%
/// do teto (cgroup já coletado antes de ler a prova).
/// **Onde:** [`super::executar`] após o `wait`.
pub fn classificar(
    codigo: Option<i32>,
    sinal: Option<i32>,
    ev: &Evidencia,
    teto_mib: u64,
) -> Veredito {
    let morto = sinal == Some(9) || codigo == Some(137);
    let falhou = morto || codigo != Some(0);
    let perto_do_teto = ev.pico_bytes.is_some_and(|b| b / (1024 * 1024) * 100 >= teto_mib * 90);
    if ev.encalhou || (ev.oom_kill > 0 || ev.result_oom) && falhou || (morto && perto_do_teto) {
        let pico_mib = ev.pico_bytes.map(|b| b / (1024 * 1024));
        return Veredito::EstourouMemoria { pico_mib, teto_mib };
    }
    match (codigo, sinal) {
        (Some(0), _) => Veredito::Concluiu(0),
        (Some(c), _) => Veredito::Falhou(c),
        (None, Some(s)) => Veredito::Falhou(128 + s),
        (None, None) => Veredito::Falhou(1),
    }
}
