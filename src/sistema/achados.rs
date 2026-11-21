//! Derivação dos [`Achado`]s a partir do [`Perfil`]: mensagens acionáveis, sem culpa, pt/en.

use crate::sistema::tipos::*;

/// Pressão de memória (some avg10, %) a partir da qual o oomd tende a agir se sem config (padrão systemd: 60%).
const PRESSAO_OOMD_PADRAO: f64 = 60.0;

/// O quê: constrói um achado bilíngue. Onde: todas as regras abaixo.
fn novo(g: Gravidade, cod: &str, m: (&str, &str), s: (&str, &str)) -> Achado {
    Achado {
        gravidade: g,
        codigo: cod.to_string(),
        mensagem: m.0.to_string(),
        sugestao: s.0.to_string(),
        mensagem_en: m.1.to_string(),
        sugestao_en: s.1.to_string(),
    }
}

/// O quê: "60%" -> 60.0. Onde: limite de pressão do oomd.
fn percentual(s: &str) -> Option<f64> {
    s.trim().trim_end_matches('%').parse::<f64>().ok().filter(|v| v.is_finite())
}

/// O quê: some.avg10 de memória, se lido. Onde: regras de PSI.
fn psi_mem_some10(p: &Perfil) -> Option<f64> {
    p.memoria.psi.memoria.get()?.some.map(|l| l.avg10)
}

/// O quê: regras de cgroup (v2, delegação, eventos, teto). Onde: `derivar`.
fn regras_cgroup(p: &Perfil, o: &mut Vec<Achado>) {
    let c = &p.cgroup;
    if c.v2.get() == Some(&false) {
        o.push(novo(Gravidade::Risco, "SYS-CGROUP-V2-AUSENTE",
            ("cgroup v2 não está ativo neste host; não há como confinar memória por execução.", "cgroup v2 is not active on this host; per-run memory confinement is unavailable."),
            ("Rode com limites de ambiente (ulimit) e o orçamento adaptativo; ative `systemd.unified_cgroup_hierarchy=1` na linha do kernel se for possível.", "Use environment limits (ulimit) plus the adaptive budget; enable `systemd.unified_cgroup_hierarchy=1` on the kernel command line if possible.")));
    }
    if let Some(d) = c.user_slice.as_ref().and_then(|u| u.delegados.get()) {
        if !d.iter().any(|x| x == "memory") {
            o.push(novo(Gravidade::Atencao, "SYS-CGROUP-SEM-MEMORY",
                ("O controlador memory não está delegado ao seu usuário; a jaula por cgroup não consegue impor teto.", "The memory controller is not delegated to your user; the cgroup cage cannot enforce a ceiling."),
                ("Peça delegação (Delegate=memory em user@.service.d) ou use `systemd-run --user --scope -p MemoryMax=...`, que depende dela.", "Ask for delegation (Delegate=memory in user@.service.d) or use `systemd-run --user --scope -p MemoryMax=...`, which depends on it.")));
        }
    }
    if c.eventos.get().and_then(|e| e.oom_kill).unwrap_or(0) > 0 {
        o.push(novo(
            Gravidade::Atencao,
            "SYS-CGROUP-OOM-KILL-PROPRIO",
            (
                "O cgroup deste processo já registrou mortes por OOM (memory.events oom_kill > 0).",
                "This process's cgroup already recorded OOM kills (memory.events oom_kill > 0).",
            ),
            (
                "Reduza o consumo por tarefa ou aumente o teto do cgroup; confira memory.events.",
                "Reduce per-task usage or raise the cgroup ceiling; check memory.events.",
            ),
        ));
    }
    if let Some(m) = c.efetivo.memory_max {
        o.push(novo(
            Gravidade::Info,
            "SYS-CGROUP-TETO-MEMORIA",
            (
                &format!("Há teto efetivo de memória no cgroup: {} MiB.", m >> 20),
                &format!("There is an effective cgroup memory ceiling: {} MiB.", m >> 20),
            ),
            (
                "O orquestrador respeita esse teto ao reservar memória (margem_segura).",
                "The orchestrator honours this ceiling when reserving memory (margem_segura).",
            ),
        ));
    }
}

/// O quê: regras dos daemons de OOM (oomd, earlyoom, múltiplos, ananicy). Onde: `derivar`.
fn regras_daemons(p: &Perfil, o: &mut Vec<Achado>) {
    let d = &p.daemons;
    if d.oomd.ativo() {
        o.push(novo(Gravidade::Info, "SYS-OOMD-ATIVO",
            ("systemd-oomd está ativo: ele encerra cgroups inteiros sob pressão de memória/swap.", "systemd-oomd is active: it kills whole cgroups under memory/swap pressure."),
            ("Rode tarefas pesadas em scope próprio (`systemd-run --user --scope`) com ManagedOOMPreference=avoid/omit e MemoryMax por unidade.", "Run heavy tasks in their own scope (`systemd-run --user --scope`) with ManagedOOMPreference=avoid/omit and per-unit MemoryMax.")));
        let lim = d
            .oomd_conf
            .get()
            .and_then(|c| c.pressao_limite.as_deref())
            .and_then(percentual)
            .unwrap_or(PRESSAO_OOMD_PADRAO);
        if psi_mem_some10(p).map(|v| v >= lim * 0.8).unwrap_or(false) {
            o.push(novo(Gravidade::Risco, "SYS-OOMD-PRESSAO-ALTA",
                ("A pressão de memória está perto do limite do systemd-oomd; ele pode matar o cgroup inteiro (todas as tarefas juntas).", "Memory pressure is near the systemd-oomd limit; it may kill the entire cgroup (all tasks together)."),
                ("Reduza a concorrência agora, isole as tarefas em scopes próprios com ManagedOOMPreference=omit/avoid e limite memória por unidade.", "Lower concurrency now, isolate tasks in their own scopes with ManagedOOMPreference=omit/avoid and cap memory per unit.")));
        }
        if d.managed_oom_user.get().and_then(|m| m.memoria.as_deref()) == Some("kill") {
            o.push(novo(Gravidade::Atencao, "SYS-OOMD-USER-SLICE-KILL",
                ("O user.slice está com ManagedOOMMemoryPressure=kill: sua sessão inteira é candidata do oomd.", "user.slice has ManagedOOMMemoryPressure=kill: your whole session is an oomd candidate."),
                ("Prefira scopes por tarefa com ManagedOOMPreference=avoid para que o alvo seja só a tarefa excedente.", "Prefer per-task scopes with ManagedOOMPreference=avoid so only the offending task is targeted.")));
        }
    }
    let n = [&d.oomd, &d.earlyoom, &d.nohang].iter().filter(|s| s.ativo()).count();
    if n > 1 {
        o.push(novo(Gravidade::Atencao, "SYS-OOM-DAEMONS-MULTIPLOS",
            ("Mais de um daemon de OOM está ativo (oomd/earlyoom/nohang); as políticas se somam e podem agir em tempos diferentes.", "More than one OOM daemon is active (oomd/earlyoom/nohang); policies add up and may act at different times."),
            ("Confira se a combinação é intencional; a margem de segurança assume o mais agressivo.", "Check the combination is intentional; the safety margin assumes the most aggressive one.")));
    }
    if d.earlyoom.ativo() {
        o.push(novo(
            Gravidade::Info,
            "SYS-EARLYOOM-ATIVO",
            (
                "earlyoom está ativo e mata o maior oom_score ao faltar memória.",
                "earlyoom is active and kills the highest oom_score when memory runs low.",
            ),
            (
                "Ajuste oom_score_adj das tarefas importantes (valor baixo) para protegê-las.",
                "Lower oom_score_adj on important tasks to protect them.",
            ),
        ));
    }
    if d.ananicy.ativo() {
        o.push(novo(Gravidade::Info, "SYS-ANANICY-ATIVO", ("ananicy-cpp está ativo e reajusta nice/ionice por nome de processo.", "ananicy-cpp is active and re-nices processes by name."),
            ("Normal no CachyOS; nada a fazer. Nomes de binário conhecidos recebem prioridade automática.", "Normal on CachyOS; nothing to do. Known binary names get automatic priority.")));
    }
}

/// O quê: regras de swap/zram/vm/PSI/memória livre/processo. Onde: `derivar`.
fn regras_memoria(p: &Perfil, o: &mut Vec<Achado>) {
    let m = &p.memoria;
    let zram_n = m.zram.get().map(Vec::len).unwrap_or(0);
    let mi = m.meminfo.get();
    if mi.and_then(|x| x.swap_total_kib) == Some(0) && zram_n == 0 {
        o.push(novo(
            Gravidade::Atencao,
            "SYS-SEM-SWAP",
            (
                "O host não tem swap nem zram; picos de memória vão direto ao OOM killer.",
                "The host has no swap or zram; memory spikes go straight to the OOM killer.",
            ),
            (
                "Considere zram (ex.: zram-generator) e mantenha uma margem maior no orçamento.",
                "Consider zram (e.g. zram-generator) and keep a larger margin in the budget.",
            ),
        ));
    }
    let sw = m.vm.swappiness.get().copied();
    if zram_n > 0 && sw.map(|s| s >= 100).unwrap_or(false) {
        o.push(novo(
            Gravidade::Info,
            "SYS-SWAPPINESS-ALTA-ZRAM",
            (
                "vm.swappiness é alto com zram; é o ajuste usual do CachyOS e é normal.",
                "vm.swappiness is high with zram; that is the usual CachyOS tuning and is normal.",
            ),
            ("Nenhuma ação necessária.", "No action needed."),
        ));
    }
    if zram_n > 0 && sw.map(|s| s <= 10).unwrap_or(false) {
        o.push(novo(Gravidade::Atencao, "SYS-SWAPPINESS-BAIXA-ZRAM", ("vm.swappiness é baixo mesmo com zram; o kernel evita usar o swap comprimido e pode atrasar a recuperação de memória.", "vm.swappiness is low even with zram; the kernel avoids compressed swap and may reclaim memory late."),
            ("Para zram, valores altos (100+) costumam ser melhores; avalie sysctl vm.swappiness.", "With zram, high values (100+) usually work better; consider sysctl vm.swappiness.")));
    }
    if m.psi.memoria.get().is_none() && m.psi.cpu.get().is_none() && m.psi.io.get().is_none() {
        o.push(novo(Gravidade::Atencao, "SYS-PSI-INDISPONIVEL", ("PSI (/proc/pressure) não está disponível; sem ele a pressão de memória não é medida.", "PSI (/proc/pressure) is unavailable; memory pressure cannot be measured."),
            ("O orçamento cai para MemAvailable; ative CONFIG_PSI (kernel) ou psi=1 na linha de boot.", "The budget falls back to MemAvailable; enable CONFIG_PSI (kernel) or psi=1 on the boot line.")));
    }
    let full = m.psi.memoria.get().and_then(|r| r.full).map(|l| l.avg10).unwrap_or(0.0);
    let some = psi_mem_some10(p).unwrap_or(0.0);
    if full >= 10.0 || some >= 40.0 {
        let g = if full >= 10.0 { Gravidade::Risco } else { Gravidade::Atencao };
        o.push(novo(
            g,
            "SYS-PSI-MEMORIA-ALTA",
            (
                "A pressão de memória está alta agora; tarefas estão esperando por memória.",
                "Memory pressure is high right now; tasks are stalling on memory.",
            ),
            (
                "Reduza a concorrência e adie novas tarefas pesadas até a pressão cair.",
                "Lower concurrency and defer new heavy tasks until pressure drops.",
            ),
        ));
    }
    if let (Some(t), Some(a)) = (mi.and_then(|x| x.total_kib), mi.and_then(|x| x.disponivel_kib)) {
        if t > 0 && a * 10 < t {
            o.push(novo(
                Gravidade::Risco,
                "SYS-MEM-DISPONIVEL-BAIXO",
                (
                    "Menos de 10% da memória está disponível (MemAvailable).",
                    "Less than 10% of memory is available (MemAvailable).",
                ),
                (
                    "Não inicie novas tarefas; libere cache/tarefas ou aguarde.",
                    "Do not start new tasks; free caches/tasks or wait.",
                ),
            ));
        }
    }
    if m.vm.overcommit_memory.get() == Some(&2) {
        o.push(novo(
            Gravidade::Atencao,
            "SYS-OVERCOMMIT-ESTRITO",
            (
                "vm.overcommit_memory=2: alocações podem falhar antes de a memória acabar.",
                "vm.overcommit_memory=2: allocations may fail before memory runs out.",
            ),
            (
                "Trate falha de alocação como erro recuperável e mantenha margem maior.",
                "Treat allocation failure as recoverable and keep a larger margin.",
            ),
        ));
    }
    if m.vm.oom_kill_allocating_task.get().map(|v| *v != 0).unwrap_or(false) {
        o.push(novo(Gravidade::Atencao, "SYS-OOM-KILL-ALLOCATING", ("vm.oom_kill_allocating_task está ligado: quem aloca na hora do OOM morre, não o maior consumidor.", "vm.oom_kill_allocating_task is on: the allocating task dies, not the biggest consumer."),
            ("Mantenha o processo orquestrador com alocação previsível e tarefas em cgroup próprio.", "Keep the orchestrator allocation-predictable and tasks in their own cgroup.")));
    }
    if p.processo.oom_score_adj.get().map(|v| *v >= 500).unwrap_or(false) {
        o.push(novo(
            Gravidade::Atencao,
            "SYS-OOM-SCORE-ADJ-ALTO",
            (
                "Este processo tem oom_score_adj alto e é candidato prioritário do OOM killer.",
                "This process has a high oom_score_adj and is a prime OOM-killer candidate.",
            ),
            (
                "Herdado do pai? Reinicie por uma unit/scope com OOMScoreAdjust=0.",
                "Inherited from the parent? Restart via a unit/scope with OOMScoreAdjust=0.",
            ),
        ));
    }
}

/// O quê: lista as leituras-chave que falharam (fonte). Onde: achado de leituras ausentes.
fn leituras_ausentes(p: &Perfil) -> Vec<String> {
    let m = &p.memoria;
    let mut v: Vec<(&str, bool)> = vec![
        (&p.cgroup.v2.fonte, p.cgroup.v2.valor.is_none()),
        (&p.cgroup.proprio.fonte, p.cgroup.proprio.valor.is_none()),
        (&m.meminfo.fonte, m.meminfo.valor.is_none()),
        (&m.psi.memoria.fonte, m.psi.memoria.valor.is_none()),
        (&m.psi.cpu.fonte, m.psi.cpu.valor.is_none()),
        (&m.psi.io.fonte, m.psi.io.valor.is_none()),
        (&m.vm.swappiness.fonte, m.vm.swappiness.valor.is_none()),
        (&p.kernel.versao.fonte, p.kernel.versao.valor.is_none()),
        (&p.kernel.distro_id.fonte, p.kernel.distro_id.valor.is_none()),
    ];
    v.retain(|(_, falhou)| *falhou);
    v.into_iter().map(|(f, _)| f.to_string()).collect()
}

/// O quê: deriva todos os achados, ordenados do mais grave ao menos grave.
/// Onde: `identificar` (uma vez, após coletar).
pub fn derivar(p: &Perfil) -> Vec<Achado> {
    let mut o = Vec::new();
    regras_cgroup(p, &mut o);
    regras_daemons(p, &mut o);
    regras_memoria(p, &mut o);
    if p.kernel.cachyos {
        o.push(novo(
            Gravidade::Info,
            "SYS-DISTRO-CACHYOS",
            (
                "Host CachyOS detectado: oomd, zram e ananicy já vêm configurados por padrão.",
                "CachyOS host detected: oomd, zram and ananicy ship preconfigured.",
            ),
            (
                "Use os padrões do sistema; não é preciso mudar nada no host.",
                "Keep system defaults; nothing to change on the host.",
            ),
        ));
    }
    let aus = leituras_ausentes(p);
    if !aus.is_empty() {
        let l = aus.join(", ");
        o.push(novo(
            Gravidade::Info,
            "SYS-LEITURAS-AUSENTES",
            (
                &format!("Algumas leituras não estavam disponíveis: {l}."),
                &format!("Some reads were unavailable: {l}."),
            ),
            (
                "O orçamento segue com o que foi lido e assume o lado seguro no resto.",
                "The budget proceeds with what was read and assumes the safe side for the rest.",
            ),
        ));
    }
    o.sort_by(|a, b| b.gravidade.cmp(&a.gravidade).then_with(|| a.codigo.cmp(&b.codigo)));
    o
}
