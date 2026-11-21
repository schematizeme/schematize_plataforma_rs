//! Testes do módulo `jaula`: funções puras (fixtures) + integração real (pula com aviso).

use super::backend::decidir;
use super::veredito::{
    classificar, parse_anon, parse_cgroup_path, parse_oom_kill, parse_result_oom, Evidencia,
};
use super::*;

const WRAP: &str = r#"echo "$0" > /proc/self/oom_score_adj 2>/dev/null; exec "$@""#;

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

#[test]
fn clamp_minimo_e_alto_exige_adr() {
    assert_eq!(Orcamento::novo(100, None).map(|o| o.mem_max_mib), Ok(500));
    assert_eq!(Orcamento::novo(2048, None).map(|o| o.mem_max_mib), Ok(2048));
    assert_eq!(Orcamento::novo(4096, None), Err(ErroOrcamento::AcimaDoTeto { pedido_mib: 4096 }));
    let o = Orcamento::novo(4096, Some("ADR-0042".into())).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((o.mem_max_mib, o.mem_high_mib, o.swap_max_mib), (4096, 3481, 0));
    assert_eq!(Orcamento::novo(600, Some(" ".into())), Err(ErroOrcamento::AdrVazio));
    assert_eq!(Orcamento::novo(1000, None).map(|o| o.mem_high_mib), Ok(850));
}

#[test]
fn estimar_por_tipo_e_jobs() {
    use TipoExecucao::*;
    let mib = |t| estimar(t).mem_max_mib;
    assert_eq!([mib(AgenteLlm), mib(Script), mib(Tool)], [1024, 512, 512]);
    assert_eq!([mib(Teste), mib(Build), mib(Navegador)], [1024, 2048, 2048]);
    assert_eq!(jobs_sugeridos(&estimar(Build)), 2);
    assert_eq!(jobs_sugeridos(&estimar(Script)), 1);
    assert_eq!(estimar(Script).swap_max_mib, 0);
}

#[test]
fn comando_systemd_completo() {
    let mut o = estimar(TipoExecucao::Teste);
    o.cpu_quota_pct = Some(150);
    let c = comando_jaulado(&o, &v(&["cargo", "test"]), "job-1");
    let prefixo = v(&["systemd-run", "--user", "--scope", "--collect", "--quiet", "--unit=job-1"]);
    let props = v(&["MemoryMax=1024M", "MemoryHigh=870M", "MemorySwapMax=0M", "TasksMax=512"]);
    let mais = v(&["OOMPolicy=kill", "CPUQuota=150%"]);
    let mut esperado = prefixo;
    for p in props.iter().chain(mais.iter()) {
        esperado.extend(v(&["-p", p]));
    }
    esperado.extend(v(&["--", "sh", "-c", WRAP, "800", "cargo", "test"]));
    assert_eq!(c, esperado);
}

#[test]
fn nome_hostil_e_sanitizado_e_argv_nao_e_tocado() {
    for hostil in ["a;rm -rf /", "x y", "ação-ü", "a\0b", "--unit=evil", ""] {
        let n = sanitizar_unidade(hostil);
        assert!(!n.is_empty() && n.len() <= 64, "{n:?}");
        assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }
    assert_eq!(sanitizar_unidade("a;b c"), "a_b_c");
    assert_eq!(sanitizar_unidade(&"z".repeat(500)).len(), 64);
    let c = comando_jaulado(&estimar(TipoExecucao::Script), &v(&["echo", "a;b $(x)"]), "n;m\0 ü");
    assert!(
        c.contains(&"--unit=n_m__".to_string()) || c.iter().any(|a| a.starts_with("--unit=n_m"))
    );
    assert_eq!(c.last().map(String::as_str), Some("a;b $(x)"));
    assert!(c.iter().any(|a| a == "echo") && c.iter().filter(|a| a.contains("exec")).count() == 1);
    assert!(c.iter().all(|a| !a.contains('\0')));
}

#[test]
fn comando_prlimit_em_bytes() {
    let c = comando_prlimit(&estimar(TipoExecucao::Script), &v(&["true"]));
    assert_eq!(c, v(&["prlimit", "--as=536870912", "--", "true"]));
}

#[test]
fn parse_memory_events_fixture() {
    let t = "low 0\nhigh 12\nmax 3\noom 1\noom_kill 1\noom_group_kill 1\n";
    assert_eq!(parse_oom_kill(t), Some(1));
    assert_eq!(parse_oom_kill("low 0\nmax 0\n"), None);
    assert_eq!(parse_oom_kill("oom_kill x\n"), None);
    assert_eq!(parse_oom_kill(""), None);
}

#[test]
fn parse_anon_fixture() {
    assert_eq!(parse_anon("anon 4096\nfile 10\nanon_thp 0\n"), Some(4096));
    assert_eq!(parse_anon("file 10\n"), None);
}

#[test]
fn encalhe_vira_estouro() {
    let ev = Evidencia { encalhou: true, pico_bytes: Some(480 << 20), ..Default::default() };
    let r = classificar(None, Some(9), &ev, 500);
    assert_eq!(r, Veredito::EstourouMemoria { pico_mib: Some(480), teto_mib: 500 });
}

#[test]
fn parse_result_e_cgroup_fixture() {
    assert!(parse_result_oom("Result=oom-kill\n"));
    assert!(!parse_result_oom("Result=success\n"));
    assert!(!parse_result_oom("Result=oom-killed\n"));
    let p = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/j.scope\n";
    assert_eq!(
        parse_cgroup_path(p).as_deref(),
        Some("/user.slice/user-1000.slice/user@1000.service/app.slice/j.scope")
    );
    assert_eq!(parse_cgroup_path("12:memory:/x\n"), None);
}

#[test]
fn classificar_veredito() {
    let ev = |k, p| Evidencia { oom_kill: k, pico_bytes: p, ..Default::default() };
    let mib = 1024 * 1024;
    assert_eq!(classificar(Some(0), None, &ev(0, None), 500), Veredito::Concluiu(0));
    assert_eq!(classificar(Some(3), None, &ev(0, None), 500), Veredito::Falhou(3));
    assert_eq!(classificar(None, Some(15), &ev(0, None), 500), Veredito::Falhou(143));
    let oom = Veredito::EstourouMemoria { pico_mib: Some(499), teto_mib: 500 };
    assert_eq!(classificar(None, Some(9), &ev(1, Some(499 * mib)), 500), oom);
    assert_eq!(classificar(Some(137), None, &ev(1, Some(499 * mib)), 500), oom);
    assert_eq!(
        classificar(None, Some(9), &ev(0, Some(480 * mib)), 500),
        Veredito::EstourouMemoria { pico_mib: Some(480), teto_mib: 500 }
    );
    // SIGKILL de outra origem, longe do teto: não é OOM.
    assert_eq!(classificar(None, Some(9), &ev(0, Some(10 * mib)), 500), Veredito::Falhou(137));
    // oom_kill de um filho, mas o principal concluiu: concluiu.
    assert_eq!(classificar(Some(0), None, &ev(1, None), 500), Veredito::Concluiu(0));
}

#[test]
fn decidir_backend_matriz() {
    let mem = Some("cpuset cpu io memory pids");
    assert_eq!(decidir(true, true, mem, true), Backend::SystemdScope);
    assert_eq!(decidir(true, true, Some("cpu io"), true), Backend::Prlimit);
    assert_eq!(decidir(true, false, mem, true), Backend::Prlimit);
    assert_eq!(decidir(false, true, None, false), Backend::Nenhum);
}

#[test]
fn argv_invalido_nao_executa() {
    if detectar_backend() == Backend::Nenhum {
        eprintln!("PULADO: sem backend de jaula nesta máquina");
        return;
    }
    let o = estimar(TipoExecucao::Script);
    assert_eq!(executar(&o, &[], "vazio"), Veredito::Falhou(127));
    assert_eq!(executar(&o, &v(&["a\0b"]), "nul"), Veredito::Falhou(127));
}

/// Integração real: aloca ~800 MiB com teto 500; o pai (este teste) segue vivo.
#[test]
fn integracao_oom_confinado_e_pai_vivo() {
    let backend = detectar_backend();
    if backend != Backend::SystemdScope {
        eprintln!("PULADO integracao_oom: backend = {backend:?} (precisa de SystemdScope)");
        return;
    }
    let pid = std::process::id();
    let aloca = "head -c 800M /dev/zero | tr '\\0' a | tail -n 1 >/dev/null";
    let mut o = Orcamento::da_classe(Classe::Leve);
    o.mem_max_mib = 500;
    o.mem_high_mib = 425;
    let r = executar(&o, &v(&["sh", "-c", aloca]), &format!("jaula-teste-oom-{pid}"));
    assert!(matches!(r, Veredito::EstourouMemoria { teto_mib: 500, .. }), "{r:?}");
    let ok = executar(&o, &v(&["true"]), &format!("jaula-teste-true-{pid}"));
    assert_eq!(ok, Veredito::Concluiu(0));
    let falha = executar(&o, &v(&["sh", "-c", "exit 3"]), &format!("jaula-teste-f-{pid}"));
    assert_eq!(falha, Veredito::Falhou(3));
    assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
}

/// Integração real: timeout mata só a unidade.
#[test]
fn integracao_timeout() {
    if detectar_backend() != Backend::SystemdScope {
        eprintln!("PULADO integracao_timeout: precisa de SystemdScope");
        return;
    }
    let mut o = estimar(TipoExecucao::Script);
    o.timeout = Some(std::time::Duration::from_millis(500));
    let n = format!("jaula-teste-to-{}", std::process::id());
    assert_eq!(executar(&o, &v(&["sleep", "30"]), &n), Veredito::Timeout);
}
