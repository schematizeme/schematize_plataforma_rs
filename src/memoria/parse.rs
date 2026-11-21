//! Parsers PUROS dos arquivos do cgroup v2 e do `/proc`. Hostil-seguros: linha inválida é
//! ignorada, arquivo sem nenhum campo reconhecido vira `Err` com motivo; nunca panic.
//!
//! **Onde:** [`super::leitura::Leitura`] (e o monitor, via leitura).

/// Valor de um limite do cgroup: o literal `max` ou bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limite {
    /// Literal `max` (sem limite).
    Max,
    /// Limite em bytes.
    Bytes(u64),
}

impl Limite {
    /// **O quê:** bytes se finito. **Onde:** projeção de tempo até o teto.
    pub fn bytes(self) -> Option<u64> {
        match self {
            Limite::Max => None,
            Limite::Bytes(b) => Some(b),
        }
    }
}

/// Campos de `memory.stat` (bytes); `None` se a linha falta/está truncada.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemStat {
    /// Memória anônima.
    pub anon: Option<u64>,
    /// Page cache.
    pub file: Option<u64>,
    /// Memória de kernel.
    pub kernel: Option<u64>,
    /// Slab.
    pub slab: Option<u64>,
    /// Shmem/tmpfs.
    pub shmem: Option<u64>,
}

/// Contadores de `memory.events`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemEventos {
    /// Reclaim abaixo de `memory.low`.
    pub low: Option<u64>,
    /// Vezes que passou de `memory.high`.
    pub high: Option<u64>,
    /// Vezes que bateu em `memory.max`.
    pub max: Option<u64>,
    /// Invocações do OOM killer.
    pub oom: Option<u64>,
    /// Processos mortos pelo OOM killer.
    pub oom_kill: Option<u64>,
}

/// Campos de `cpu.stat`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuStat {
    /// CPU consumida (µs).
    pub usage_usec: Option<u64>,
    /// Períodos com throttle.
    pub nr_throttled: Option<u64>,
}

/// Resumo de `io.stat` somado entre dispositivos (saturando).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoResumo {
    /// Bytes lidos.
    pub rbytes: u64,
    /// Bytes escritos.
    pub wbytes: u64,
    /// Operações de leitura.
    pub rios: u64,
    /// Operações de escrita.
    pub wios: u64,
    /// Dispositivos com linha válida.
    pub dispositivos: u32,
}

/// Uma linha PSI (`some`/`full`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsiLinha {
    /// Média de 10 s (%).
    pub avg10: f64,
    /// Média de 60 s (%).
    pub avg60: f64,
    /// Média de 300 s (%).
    pub avg300: f64,
    /// Tempo total estagnado (µs).
    pub total_us: u64,
}

/// PSI de um recurso. `full` falta em `cpu.pressure` de kernels antigos.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Psi {
    /// Algum task parado.
    pub some: Option<PsiLinha>,
    /// Todos os tasks parados.
    pub full: Option<PsiLinha>,
}

/// Campos de `/proc/meminfo` em bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemInfo {
    /// `MemTotal`.
    pub total: Option<u64>,
    /// `MemAvailable`.
    pub disponivel: Option<u64>,
}

/// **O quê:** primeiro valor `u64` de um arquivo de valor único (`memory.current`).
/// **Onde:** `memory.current/peak`, `memory.swap.current`, `pids.current`.
pub fn parse_u64(t: &str) -> Result<u64, String> {
    let s = t.trim();
    s.parse().map_err(|_| format!("valor inválido {:?}", amostra(s)))
}

/// **O quê:** `max` literal ou bytes. **Onde:** `memory.max`, `memory.high`.
pub fn parse_limite(t: &str) -> Result<Limite, String> {
    let s = t.trim();
    if s == "max" {
        return Ok(Limite::Max);
    }
    s.parse().map(Limite::Bytes).map_err(|_| format!("limite inválido {:?}", amostra(s)))
}

/// Primeiros 24 caracteres, para mensagens de erro sem ecoar lixo gigante.
fn amostra(s: &str) -> String {
    s.chars().take(24).flat_map(char::escape_debug).collect()
}

/// **O quê:** valor da primeira linha `chave <u64>` (linha truncada/inválida é ignorada).
/// **Onde:** parsers chave-valor planos.
pub fn campo_kv(t: &str, chave: &str) -> Option<u64> {
    t.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        if it.next()? != chave {
            return None;
        }
        let v = it.next()?;
        if it.next().is_some() {
            return None;
        }
        v.parse().ok()
    })
}

/// **O quê:** `memory.stat` → [`MemStat`]; `Err` se nenhum dos 5 campos existe.
/// **Onde:** [`super::leitura::Leitura`].
pub fn parse_mem_stat(t: &str) -> Result<MemStat, String> {
    let s = MemStat {
        anon: campo_kv(t, "anon"),
        file: campo_kv(t, "file"),
        kernel: campo_kv(t, "kernel"),
        slab: campo_kv(t, "slab"),
        shmem: campo_kv(t, "shmem"),
    };
    if s == MemStat::default() {
        return Err("memory.stat sem campos reconhecidos".to_string());
    }
    Ok(s)
}

/// **O quê:** `memory.events` → [`MemEventos`]; `Err` se nada reconhecido.
/// **Onde:** [`super::leitura::Leitura`] e detecção de OOM/high.
pub fn parse_mem_eventos(t: &str) -> Result<MemEventos, String> {
    let e = MemEventos {
        low: campo_kv(t, "low"),
        high: campo_kv(t, "high"),
        max: campo_kv(t, "max"),
        oom: campo_kv(t, "oom"),
        oom_kill: campo_kv(t, "oom_kill"),
    };
    if e == MemEventos::default() {
        return Err("memory.events sem campos reconhecidos".to_string());
    }
    Ok(e)
}

/// **O quê:** `cpu.stat` → [`CpuStat`]; `Err` se nada reconhecido. **Onde:** leitura completa.
pub fn parse_cpu_stat(t: &str) -> Result<CpuStat, String> {
    let c = CpuStat {
        usage_usec: campo_kv(t, "usage_usec"),
        nr_throttled: campo_kv(t, "nr_throttled"),
    };
    if c == CpuStat::default() {
        return Err("cpu.stat sem campos reconhecidos".to_string());
    }
    Ok(c)
}

/// **O quê:** soma `rbytes/wbytes/rios/wios` das linhas `MAJ:MIN k=v ...`; vazio é válido (sem
/// I/O); não vazio sem nenhuma linha válida é `Err`. **Onde:** leitura completa.
pub fn parse_io_stat(t: &str) -> Result<IoResumo, String> {
    let mut r = IoResumo::default();
    let mut nao_vazias = 0u32;
    for linha in t.lines().filter(|l| !l.trim().is_empty()) {
        nao_vazias += 1;
        let mut it = linha.split_whitespace();
        if !it.next().is_some_and(|d| d.contains(':')) {
            continue;
        }
        let mut achou = false;
        for kv in it {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let Ok(v) = v.parse::<u64>() else { continue };
            achou |= acumular_io(&mut r, k, v);
        }
        if achou {
            r.dispositivos = r.dispositivos.saturating_add(1);
        }
    }
    if nao_vazias > 0 && r.dispositivos == 0 {
        return Err("io.stat sem linha válida".to_string());
    }
    Ok(r)
}

/// Soma `v` no campo de `r` chamado `k`; `true` se `k` é um dos quatro conhecidos.
fn acumular_io(r: &mut IoResumo, k: &str, v: u64) -> bool {
    let alvo = match k {
        "rbytes" => &mut r.rbytes,
        "wbytes" => &mut r.wbytes,
        "rios" => &mut r.rios,
        "wios" => &mut r.wios,
        _ => return false,
    };
    *alvo = alvo.saturating_add(v);
    true
}

/// **O quê:** `*.pressure`/`/proc/pressure/*` → [`Psi`]. Linha com NaN/inf/negativo/>100 ou
/// campo faltando é descartada; `Err` se nem `some` nem `full` sobrou. **Onde:** leitura.
pub fn parse_psi(t: &str) -> Result<Psi, String> {
    let mut psi = Psi::default();
    for l in t.lines() {
        let mut it = l.split_whitespace();
        match it.next() {
            Some("some") => psi.some = parse_psi_linha(it).or(psi.some),
            Some("full") => psi.full = parse_psi_linha(it).or(psi.full),
            _ => {}
        }
    }
    if psi.some.is_none() && psi.full.is_none() {
        return Err("PSI sem linha válida".to_string());
    }
    Ok(psi)
}

/// Lê `avg10= avg60= avg300= total=` do resto de uma linha PSI.
fn parse_psi_linha<'a>(it: impl Iterator<Item = &'a str>) -> Option<PsiLinha> {
    let (mut a10, mut a60, mut a300, mut total) = (None, None, None, None);
    for kv in it {
        let (k, v) = kv.split_once('=')?;
        match k {
            "avg10" => a10 = pct(v),
            "avg60" => a60 = pct(v),
            "avg300" => a300 = pct(v),
            "total" => total = v.parse().ok(),
            _ => {}
        }
    }
    Some(PsiLinha { avg10: a10?, avg60: a60?, avg300: a300?, total_us: total? })
}

/// Percentual finito em `[0, 100]`, senão `None`.
fn pct(v: &str) -> Option<f64> {
    v.parse::<f64>().ok().filter(|x| x.is_finite() && (0.0..=100.0).contains(x))
}

/// **O quê:** `/proc/meminfo` → [`MemInfo`] (kB→bytes, saturando em `None` no overflow);
/// `Err` se faltam ambos os campos. **Onde:** leitura do host e [`super::observador`].
pub fn parse_meminfo(t: &str) -> Result<MemInfo, String> {
    let m = MemInfo {
        total: meminfo_campo(t, "MemTotal:"),
        disponivel: meminfo_campo(t, "MemAvailable:"),
    };
    if m == MemInfo::default() {
        return Err("meminfo sem MemTotal/MemAvailable".to_string());
    }
    Ok(m)
}

/// Valor em bytes da linha `<chave> <n> kB` de meminfo.
fn meminfo_campo(t: &str, chave: &str) -> Option<u64> {
    t.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        if it.next()? != chave {
            return None;
        }
        let kib: u64 = it.next()?.parse().ok()?;
        match it.next() {
            Some("kB") | None => kib.checked_mul(1024),
            Some(_) => None,
        }
    })
}

/// **O quê:** caminho relativo do cgroup v2 em `/proc/<pid>/cgroup` (`0::/x/y`), recusando `..`
/// e caminhos sem `/` inicial; sufixo ` (deleted)` é removido. **Onde:** resolver `Alvo::Pid`.
pub fn parse_proc_cgroup(t: &str) -> Result<String, String> {
    let p = t
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .ok_or_else(|| "sem linha cgroup v2 (0::)".to_string())?;
    let p = p.trim().trim_end_matches(" (deleted)");
    if !p.starts_with('/') || p.split('/').any(|c| c == "..") || p.contains('\0') {
        return Err(format!("caminho de cgroup recusado {:?}", amostra(p)));
    }
    Ok(p.to_string())
}
