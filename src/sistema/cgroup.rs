//! Coleta de cgroup v2: ativo?, controladores, cgroup do processo, limites efetivos subindo a
//! hierarquia, memory.events e o `user@UID.service`. Somente leitura.

use crate::sistema::erro::ErroSistema;
use crate::sistema::fonte::Fonte;
use crate::sistema::parse::*;
use crate::sistema::tipos::*;

/// Raiz do cgroup v2 unificado.
pub const RAIZ: &str = "/sys/fs/cgroup";

/// O quê: lê um arquivo de limite do nível; falha vira `None` + motivo em `motivos`.
/// Onde: `ler_nivel`.
fn ler_limite(f: &dyn Fonte, dir: &str, nome: &str, motivos: &mut Vec<String>) -> Option<Limite> {
    let p = format!("{dir}/{nome}");
    match f.ler_arquivo(&p).and_then(|t| parse_limite(&t, nome)) {
        Ok(l) => Some(l),
        Err(e) => {
            motivos.push(format!("{nome}: {e}"));
            None
        }
    }
}

/// O quê: lê os limites e o consumo de um cgroup (caminho relativo a [`RAIZ`]).
/// Onde: `coletar` (níveis do próprio) e user slice.
pub fn ler_nivel(f: &dyn Fonte, rel: &str) -> NivelCgroup {
    let dir = format!("{RAIZ}{rel}");
    let mut motivos = Vec::new();
    let memory_current = match f
        .ler_arquivo(&format!("{dir}/memory.current"))
        .and_then(|t| parse_u64(&t, "memory.current"))
    {
        Ok(v) => Some(v),
        Err(e) => {
            motivos.push(format!("memory.current: {e}"));
            None
        }
    };
    NivelCgroup {
        caminho: rel.to_string(),
        memory_max: ler_limite(f, &dir, "memory.max", &mut motivos),
        memory_high: ler_limite(f, &dir, "memory.high", &mut motivos),
        swap_max: ler_limite(f, &dir, "memory.swap.max", &mut motivos),
        pids_max: ler_limite(f, &dir, "pids.max", &mut motivos),
        memory_current,
        motivos,
    }
}

/// O quê: caminhos do próprio até o topo, sem a raiz (`/a/b` -> `[/a/b, /a]`).
/// Onde: `coletar`.
pub fn niveis_acima(rel: &str) -> Vec<String> {
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    (1..=parts.len()).rev().map(|n| format!("/{}", parts[..n].join("/"))).collect()
}

/// O quê: menor valor numérico entre os candidatos. Onde: `efetivo`.
fn minimo(it: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    it.flatten().min()
}

/// O quê: calcula limites efetivos (mínimo numérico) e a menor folga. Onde: `coletar`.
pub fn efetivo(niveis: &[NivelCgroup]) -> Efetivo {
    let teto = |l: &Option<Limite>| l.and_then(Limite::teto);
    let folga = niveis.iter().map(|n| {
        let cap = minimo([teto(&n.memory_max), teto(&n.memory_high)].into_iter())?;
        Some(cap.saturating_sub(n.memory_current?))
    });
    Efetivo {
        memory_max: minimo(niveis.iter().map(|n| teto(&n.memory_max))),
        memory_high: minimo(niveis.iter().map(|n| teto(&n.memory_high))),
        swap_max: minimo(niveis.iter().map(|n| teto(&n.swap_max))),
        pids_max: minimo(niveis.iter().map(|n| teto(&n.pids_max))),
        folga_bytes: minimo(folga),
    }
}

/// O quê: v2 ativo + controladores da raiz a partir de `cgroup.controllers`.
/// Onde: `coletar`.
fn ler_v2(f: &dyn Fonte) -> (Lido<bool>, Lido<Vec<String>>) {
    let p = format!("{RAIZ}/cgroup.controllers");
    match f.ler_arquivo(&p) {
        Ok(t) => (Lido::ok(true, &p), Lido::ok(parse_controladores(&t), &p)),
        Err(ErroSistema::Ausente(_)) => (Lido::ok(false, &p), Lido::ok(Vec::new(), &p)),
        Err(e) => (Lido::falha(&p, &e), Lido::falha(&p, e)),
    }
}

/// O quê: user@UID.service (delegados + limites). Onde: `coletar`.
fn user_slice(f: &dyn Fonte, uid: u32) -> UserSlice {
    let rel = format!("/user.slice/user-{uid}.slice/user@{uid}.service");
    let p = format!("{RAIZ}{rel}/cgroup.controllers");
    let delegados = Lido::de(&p, f.ler_arquivo(&p).map(|t| parse_controladores(&t)));
    UserSlice { nivel: ler_nivel(f, &rel), caminho: rel, delegados }
}

/// O quê: coleta completa de cgroup. `uid` vem do coletor de kernel.
/// Onde: `identificar`.
pub fn coletar(f: &dyn Fonte, uid: Option<u32>) -> Cgroup {
    let (v2, controladores) = ler_v2(f);
    let pp = "/proc/self/cgroup";
    let proprio = Lido::de(pp, f.ler_arquivo(pp).and_then(|t| parse_cgroup_proprio(&t)));
    let ativo = v2.get().copied().unwrap_or(false);
    let rel = proprio.get().filter(|_| ativo).cloned();
    let niveis: Vec<NivelCgroup> = rel
        .as_deref()
        .map(|r| niveis_acima(r).iter().map(|c| ler_nivel(f, c)).collect())
        .unwrap_or_default();
    let ev = rel.as_deref().map(|r| format!("{RAIZ}{r}/memory.events"));
    let eventos = match &ev {
        Some(p) => Lido::de(p, f.ler_arquivo(p).and_then(|t| parse_eventos(&t))),
        None => Lido::falha("memory.events", "cgroup v2 ou cgroup do processo indisponível"),
    };
    Cgroup {
        efetivo: efetivo(&niveis),
        user_slice: uid.filter(|_| ativo).map(|u| user_slice(f, u)),
        v2,
        controladores,
        proprio,
        niveis,
        eventos,
    }
}
