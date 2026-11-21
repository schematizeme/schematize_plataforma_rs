//! Resumo textual do [`Perfil`] em pt/en.

use crate::sistema::tipos::*;
use std::fmt::Display;

/// O quê: escolhe o texto do idioma. Onde: todo o resumo.
fn t<'a>(i: Idioma, pt: &'a str, en: &'a str) -> &'a str {
    match i {
        Idioma::Pt => pt,
        Idioma::En => en,
    }
}

/// O quê: valor ou "?" (com a fonte omitida; o motivo vai no JSON). Onde: linhas do resumo.
fn v<T: Display>(l: &Lido<T>) -> String {
    l.get().map(|x| x.to_string()).unwrap_or_else(|| "?".to_string())
}

/// O quê: bool em sim/não ou yes/no. Onde: linhas do resumo.
fn sn(i: Idioma, b: bool) -> &'static str {
    match (i, b) {
        (Idioma::Pt, true) => "sim",
        (Idioma::Pt, false) => "não",
        (Idioma::En, true) => "yes",
        (Idioma::En, false) => "no",
    }
}

/// O quê: formata `Option<u64>` bytes como MiB ou "sem teto". Onde: cgroup.
fn mib(i: Idioma, b: Option<u64>) -> String {
    b.map(|x| format!("{} MiB", x >> 20))
        .unwrap_or_else(|| t(i, "sem teto", "no ceiling").to_string())
}

/// O quê: texto de PSI some/full avg10. Onde: seção PSI.
fn psi(r: &Lido<PsiRecurso>) -> String {
    match r.get() {
        None => "?".to_string(),
        Some(p) => {
            let f = |l: Option<PsiLinha>| {
                l.map(|x| format!("{:.2}", x.avg10)).unwrap_or_else(|| "-".into())
            };
            format!("some {} / full {}", f(p.some), f(p.full))
        }
    }
}

/// O quê: nome da gravidade. Onde: lista de achados.
fn grav(g: Gravidade) -> &'static str {
    match g {
        Gravidade::Info => "INFO",
        Gravidade::Atencao => "ATENCAO",
        Gravidade::Risco => "RISCO",
    }
}

/// O quê: seção de memória do resumo. Onde: `resumo`.
fn sec_memoria(p: &Perfil, i: Idioma, o: &mut String) {
    let m = &p.memoria;
    let mi = m.meminfo.get().cloned().unwrap_or_default();
    let k = |x: Option<u64>| x.map(|n| (n / 1024).to_string()).unwrap_or_else(|| "?".into());
    o.push_str(&format!(
        "{}: total {} MiB, {} {} MiB, swap {}/{} MiB\n",
        t(i, "Memória", "Memory"),
        k(mi.total_kib),
        t(i, "disponível", "available"),
        k(mi.disponivel_kib),
        k(mi.swap_livre_kib),
        k(mi.swap_total_kib)
    ));
    o.push_str(&format!(
        "zram: {}\n",
        m.zram
            .get()
            .map(|z| z
                .iter()
                .map(|d| format!(
                    "{}={}/{}",
                    d.nome,
                    d.algoritmo.as_deref().unwrap_or("?"),
                    d.disksize_bytes
                        .map(|b| format!("{} MiB", b >> 20))
                        .unwrap_or_else(|| "?".into())
                ))
                .collect::<Vec<_>>()
                .join(", "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| t(i, "nenhum", "none").to_string())
    ));
    o.push_str(&format!("vm: swappiness={} overcommit={} min_free_kbytes={} page-cluster={} vfs_cache_pressure={} watermark_scale_factor={} oom_kill_allocating_task={}\n", v(&m.vm.swappiness), v(&m.vm.overcommit_memory), v(&m.vm.min_free_kbytes), v(&m.vm.page_cluster), v(&m.vm.vfs_cache_pressure), v(&m.vm.watermark_scale_factor), v(&m.vm.oom_kill_allocating_task)));
    o.push_str(&format!(
        "PSI: mem [{}] cpu [{}] io [{}]\n",
        psi(&m.psi.memoria),
        psi(&m.psi.cpu),
        psi(&m.psi.io)
    ));
}

/// O quê: seção de cgroup do resumo. Onde: `resumo`.
fn sec_cgroup(p: &Perfil, i: Idioma, o: &mut String) {
    let c = &p.cgroup;
    o.push_str(&format!(
        "cgroup v2: {} ({}) {}\n",
        c.v2.get().map(|b| sn(i, *b)).unwrap_or("?"),
        c.proprio.get().map(String::as_str).unwrap_or("?"),
        c.controladores.get().map(|x| x.join(",")).unwrap_or_default()
    ));
    o.push_str(&format!(
        "{}: memory.max {}, memory.high {}, swap.max {}, pids.max {}\n",
        t(i, "Limites efetivos", "Effective limits"),
        mib(i, c.efetivo.memory_max),
        mib(i, c.efetivo.memory_high),
        mib(i, c.efetivo.swap_max),
        c.efetivo
            .pids_max
            .map(|n| n.to_string())
            .unwrap_or_else(|| t(i, "sem teto", "no ceiling").into())
    ));
    if let Some(e) = c.eventos.get() {
        o.push_str(&format!(
            "memory.events: oom={} oom_kill={} high={} max={}\n",
            e.oom.unwrap_or(0),
            e.oom_kill.unwrap_or(0),
            e.high.unwrap_or(0),
            e.max.unwrap_or(0)
        ));
    }
}

/// O quê: seção de daemons/kernel do resumo. Onde: `resumo`.
fn sec_sistema(p: &Perfil, i: Idioma, o: &mut String) {
    let d = &p.daemons;
    let s = |x: &Servico| format!("{}={}", x.unidade, sn(i, x.ativo()));
    o.push_str(&format!(
        "{}: {}, {}, {}, {}, {}\n",
        t(i, "Daemons", "Daemons"),
        s(&d.oomd),
        s(&d.earlyoom),
        s(&d.nohang),
        s(&d.ananicy),
        s(&d.power_profiles)
    ));
    if let Some(c) = d.oomd_conf.get() {
        o.push_str(&format!("oomd.conf: SwapUsedLimit={} DefaultMemoryPressureLimit={} DefaultMemoryPressureDurationSec={}\n", c.swap_used_limit.as_deref().unwrap_or("-"), c.pressao_limite.as_deref().unwrap_or("-"), c.pressao_duracao.as_deref().unwrap_or("-")));
    }
    o.push_str(&format!(
        "Kernel: {} | distro: {}{} | CPUs: {} | oom_score {} adj {}\n",
        v(&p.kernel.osrelease),
        v(&p.kernel.distro_id),
        if p.kernel.cachyos { " (cachyos)" } else { "" },
        p.kernel.cpus.map(|n| n.to_string()).unwrap_or_else(|| "?".into()),
        v(&p.processo.oom_score),
        v(&p.processo.oom_score_adj)
    ));
}

/// O quê: texto legível do perfil e dos achados no idioma pedido.
/// Onde: `Perfil::resumo` (CLIs/GUIs/logs).
pub fn resumo(p: &Perfil, i: Idioma) -> String {
    let mut o = String::new();
    o.push_str(t(i, "== Perfil de memória/OOM do host ==\n", "== Host memory/OOM profile ==\n"));
    sec_cgroup(p, i, &mut o);
    sec_memoria(p, i, &mut o);
    sec_sistema(p, i, &mut o);
    o.push_str(&format!("{} ({}):\n", t(i, "Achados", "Findings"), p.achados.len()));
    for a in &p.achados {
        let (m, s) = match i {
            Idioma::Pt => (&a.mensagem, &a.sugestao),
            Idioma::En => (&a.mensagem_en, &a.sugestao_en),
        };
        o.push_str(&format!(
            "- [{}] {}: {}\n    {} {}\n",
            grav(a.gravidade),
            a.codigo,
            m,
            t(i, "Sugestão:", "Suggestion:"),
            s
        ));
    }
    o
}
