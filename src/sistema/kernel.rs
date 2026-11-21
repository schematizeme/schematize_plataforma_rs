//! Coleta de kernel, distro, CPUs, UID e oom_score do próprio processo. Somente leitura.

use crate::sistema::fonte::Fonte;
use crate::sistema::parse::*;
use crate::sistema::tipos::*;

/// O quê: lê arquivo de texto de uma linha, saneado. Onde: `coletar`.
fn texto(f: &dyn Fonte, p: &str) -> Lido<String> {
    Lido::de(p, f.ler_arquivo(p).map(|t| sanear(&t)))
}

/// O quê: kernel + distro (marca `cachyos`) + CPUs + UID. Onde: `identificar`.
pub fn coletar(f: &dyn Fonte) -> Kernel {
    let po = "/etc/os-release";
    let os = f.ler_arquivo(po).and_then(|t| parse_os_release(&t));
    let (distro_id, id_like) = match os {
        Ok((id, like)) => (Lido::ok(id, po), like),
        Err(e) => (Lido::falha(po, e), Vec::new()),
    };
    let cachyos = distro_id.get().map(|i| i.contains("cachyos")).unwrap_or(false)
        || id_like.iter().any(|l| l.contains("cachyos"));
    let pu = "/proc/self/status";
    Kernel {
        versao: texto(f, "/proc/version"),
        osrelease: texto(f, "/proc/sys/kernel/osrelease"),
        distro_id,
        id_like,
        cachyos,
        cpus: f.cpus(),
        uid: Lido::de(pu, f.ler_arquivo(pu).and_then(|t| parse_uid(&t))),
    }
}

/// O quê: oom_score e oom_score_adj do próprio processo. Onde: `identificar`.
pub fn coletar_processo(f: &dyn Fonte) -> ProcessoOom {
    let l = |p: &str, c: &str| Lido::de(p, f.ler_arquivo(p).and_then(|t| parse_i64(&t, c)));
    ProcessoOom {
        oom_score: l("/proc/self/oom_score", "oom_score"),
        oom_score_adj: l("/proc/self/oom_score_adj", "oom_score_adj"),
    }
}
