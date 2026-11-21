//! Montagem PURA da linha de comando jaulada (argv puro, sem shell).
//!
//! **O quê:** [`comando_jaulado`] (systemd), [`comando_prlimit`] e [`sanitizar_unidade`].
//! **Onde:** [`super::executar`] roda o vetor; testes comparam o vetor.

use super::orcamento::Orcamento;

/// Tamanho máximo do nome de unidade (o limite do systemd é 256; sobra margem).
const NOME_MAX: usize = 64;

/// **O quê:** sanitiza o nome de unidade: só `[a-zA-Z0-9_-]`, o resto vira `_`, no máx. 64
/// caracteres; vazio vira `jaula`. Garante que nada vire opção/injeção no `--unit=`.
/// **Onde:** [`comando_jaulado`].
pub fn sanitizar_unidade(nome: &str) -> String {
    let limpo: String = nome
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .take(NOME_MAX)
        .collect();
    if limpo.is_empty() {
        "jaula".to_string()
    } else {
        limpo
    }
}

/// **O quê:** argv `systemd-run --user --scope ... -- argv...` com MemoryMax/High, swap 0,
/// TasksMax, OOMPolicy=kill e CPUQuota opcional; o `oom_score_adj` entra via wrapper `sh`
/// (ver `com_oom_score`). Nenhuma string de shell.
/// **Onde:** [`super::executar`] (backend `SystemdScope`).
pub fn comando_jaulado(o: &Orcamento, argv: &[String], nome_unidade: &str) -> Vec<String> {
    let mut c: Vec<String> = ["systemd-run", "--user", "--scope", "--collect", "--quiet"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    c.push(format!("--unit={}", sanitizar_unidade(nome_unidade)));
    let props = [
        format!("MemoryMax={}M", o.mem_max_mib),
        format!("MemoryHigh={}M", o.mem_high_mib),
        format!("MemorySwapMax={}M", o.swap_max_mib),
        format!("TasksMax={}", o.pids_max),
        "OOMPolicy=kill".to_string(),
    ];
    for p in props {
        c.push("-p".into());
        c.push(p);
    }
    if let Some(pct) = o.cpu_quota_pct {
        c.push("-p".into());
        c.push(format!("CPUQuota={pct}%"));
    }
    c.push("--".into());
    c.extend(com_oom_score(o, argv));
    c
}

/// **O quê:** o scope do systemd NÃO aceita `OOMScoreAdjust=` (propriedade de exec), então um
/// `sh` mínimo, com argv puro (o valor e o comando vão como parâmetros posicionais, nunca
/// interpolados no script), grava o score em `/proc/self/oom_score_adj` e faz `exec`. Subir o
/// próprio score é permitido sem privilégio; se falhar, o `MemoryMax` segue valendo.
/// **Onde:** [`comando_jaulado`].
fn com_oom_score(o: &Orcamento, argv: &[String]) -> Vec<String> {
    let script = r#"echo "$0" > /proc/self/oom_score_adj 2>/dev/null; exec "$@""#;
    let mut v: Vec<String> = vec!["sh".into(), "-c".into(), script.into()];
    v.push(o.oom_score_adj.to_string());
    v.extend(argv.iter().cloned());
    v
}

/// **O quê:** argv `prlimit --as=<bytes> -- argv...` (fallback por processo, via RLIMIT_AS).
/// **Onde:** [`super::executar`] (backend `Prlimit`).
pub fn comando_prlimit(o: &Orcamento, argv: &[String]) -> Vec<String> {
    let mut c = vec!["prlimit".to_string(), format!("--as={}", o.mem_max_mib * 1024 * 1024)];
    c.push("--".into());
    c.extend(argv.iter().cloned());
    c
}
