//! Testes de `sistema` com hosts falsos: CachyOS, sem cgroup v2 e hostil.
#![cfg(feature = "sistema")]

use schematize_plataforma::sistema::fixtures::{cachyos, hostil, sem_cgroup_v2};
use schematize_plataforma::sistema::parse::*;
use schematize_plataforma::sistema::*;

fn tem(p: &Perfil, cod: &str) -> Option<Gravidade> {
    p.achados.iter().find(|a| a.codigo == cod).map(|a| a.gravidade)
}

#[test]
fn cachyos_le_tudo() {
    let p = identificar(&cachyos());
    assert!(p.kernel.cachyos);
    assert_eq!(p.kernel.cpus, Some(16));
    assert_eq!(p.kernel.uid.valor, Some(1000));
    assert_eq!(p.cgroup.v2.valor, Some(true));
    assert!(p.cgroup.controladores.get().unwrap().contains(&"memory".to_string()));
    assert_eq!(p.cgroup.niveis.len(), 5);
    assert_eq!(p.cgroup.efetivo.memory_max, Some(8_589_934_592));
    assert_eq!(p.cgroup.efetivo.folga_bytes, Some(8_589_934_592 - 1_073_741_824));
    assert_eq!(p.cgroup.eventos.get().unwrap().oom_kill, Some(0));
    let z = &p.memoria.zram.get().unwrap()[0];
    assert_eq!(
        (z.algoritmo.as_deref(), z.prioridade, z.disksize_bytes),
        (Some("zstd"), Some(100), Some(17179869184))
    );
    assert_eq!(p.memoria.vm.swappiness.valor, Some(150));
    assert_eq!(p.memoria.psi.memoria.get().unwrap().some.unwrap().avg10, 0.5);
    assert!(
        p.daemons.oomd.ativo() && p.daemons.ananicy.ativo() && p.daemons.power_profiles.ativo()
    );
    assert!(!p.daemons.earlyoom.ativo());
    let c = p.daemons.oomd_conf.get().unwrap();
    assert_eq!(
        (c.swap_used_limit.as_deref(), c.pressao_limite.as_deref(), c.pressao_duracao.as_deref()),
        (Some("90%"), Some("60%"), Some("20s"))
    );
    assert_eq!(p.daemons.managed_oom_user.get().unwrap().memoria.as_deref(), Some("kill"));
    let ac = p.daemons.accounting.get().unwrap();
    assert_eq!((ac.tarefas.as_deref(), ac.memoria.as_deref()), (Some("yes"), None));
    assert_eq!(p.processo.oom_score_adj.valor, Some(200));
}

#[test]
fn cachyos_achados() {
    let p = identificar(&cachyos());
    assert_eq!(tem(&p, "SYS-OOMD-ATIVO"), Some(Gravidade::Info));
    assert_eq!(tem(&p, "SYS-SWAPPINESS-ALTA-ZRAM"), Some(Gravidade::Info));
    assert_eq!(tem(&p, "SYS-DISTRO-CACHYOS"), Some(Gravidade::Info));
    assert_eq!(tem(&p, "SYS-OOMD-USER-SLICE-KILL"), Some(Gravidade::Atencao));
    assert!(tem(&p, "SYS-OOMD-PRESSAO-ALTA").is_none());
    assert!(tem(&p, "SYS-SEM-SWAP").is_none() && tem(&p, "SYS-PSI-INDISPONIVEL").is_none());
    assert!(p.achados.windows(2).all(|w| w[0].gravidade >= w[1].gravidade));
}

#[test]
fn cachyos_pressao_alta_com_oomd_vira_risco() {
    let f = cachyos().arquivo("/proc/pressure/memory", "some avg10=55.00 avg60=30.0 avg300=10.0 total=1\nfull avg10=12.0 avg60=1.0 avg300=1.0 total=1\n");
    let p = identificar(&f);
    assert_eq!(tem(&p, "SYS-OOMD-PRESSAO-ALTA"), Some(Gravidade::Risco));
    assert_eq!(tem(&p, "SYS-PSI-MEMORIA-ALTA"), Some(Gravidade::Risco));
    let a = p.achados.iter().find(|a| a.codigo == "SYS-OOMD-PRESSAO-ALTA").unwrap();
    assert!(
        a.sugestao.contains("ManagedOOMPreference")
            && a.sugestao_en.contains("ManagedOOMPreference")
    );
}

#[test]
fn cachyos_margem_respeita_cgroup() {
    let m = margem_segura(&identificar(&cachyos()));
    assert_eq!(m.disponivel_host_mib, 16000);
    assert_eq!(m.piso_critico_mib, 3200);
    assert_eq!(m.folga_cgroup_mib, Some(7168));
    assert_eq!((m.reservavel_mib, m.limitado_por), (7168, LimitadoPor::Cgroup));
}

#[test]
fn margem_host_quando_sem_teto_e_desconhecida_sem_meminfo() {
    let f = cachyos().arquivo("/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/app-x.scope/memory.max", "max\n");
    let m = margem_segura(&identificar(&f));
    assert_eq!(m.limitado_por, LimitadoPor::Host);
    assert_eq!(m.reservavel_mib, 16000 - 3200);
    let vazio = margem_segura(&identificar(&FonteFalsa::nova()));
    assert_eq!((vazio.reservavel_mib, vazio.limitado_por), (0, LimitadoPor::Desconhecido));
}

#[test]
fn sem_cgroup_v2_degrada() {
    let p = identificar(&sem_cgroup_v2());
    assert_eq!(p.cgroup.v2.valor, Some(false));
    assert!(p.cgroup.proprio.valor.is_none() && p.cgroup.proprio.motivo.is_some());
    assert!(p.cgroup.niveis.is_empty() && p.cgroup.user_slice.is_none());
    assert_eq!(tem(&p, "SYS-CGROUP-V2-AUSENTE"), Some(Gravidade::Risco));
    assert_eq!(tem(&p, "SYS-SEM-SWAP"), Some(Gravidade::Atencao));
    assert_eq!(tem(&p, "SYS-PSI-INDISPONIVEL"), Some(Gravidade::Atencao));
    assert!(tem(&p, "SYS-LEITURAS-AUSENTES").is_some());
    assert!(!p.daemons.oomd.ativo());
    assert!(p.daemons.oomd.active_state.motivo.is_some());
    assert_eq!(margem_segura(&p).reservavel_mib, 3906 - 781);
}

#[test]
fn hostil_nao_quebra_e_registra_motivo() {
    let p = identificar(&hostil());
    assert!(p.memoria.meminfo.valor.is_none() && p.memoria.meminfo.motivo.is_some());
    assert!(
        p.memoria.vm.swappiness.valor.is_none() && p.memoria.vm.min_free_kbytes.valor.is_none()
    );
    assert!(p.memoria.psi.memoria.get().is_none() && p.memoria.psi.cpu.get().is_none());
    assert!(p.cgroup.proprio.valor.is_none());
    assert!(p.processo.oom_score_adj.valor.is_none());
    assert!(p.kernel.uid.valor.is_none());
    assert!(!p.daemons.earlyoom.ativo(), "/proc/abc não é pid");
    assert_eq!(p.daemons.oomd_conf.get().unwrap().swap_used_limit.as_deref(), Some("95%"));
    let zs = p.memoria.zram.get().unwrap();
    assert_eq!(zs.len(), 1, "zram9x rejeitado");
    assert!(zs[0].disksize_bytes.is_none() && zs[0].algoritmo.is_none());
    let id = p.kernel.distro_id.get().unwrap();
    assert!(!id.contains('\0') && p.kernel.id_like.iter().all(|s| !s.contains('\0')));
    let _ = p.resumo(Idioma::Pt);
}

#[test]
fn json_valido_sem_controles_e_resumo_bilingue() {
    for f in [cachyos(), sem_cgroup_v2(), hostil(), FonteFalsa::nova()] {
        let p = identificar(&f);
        let j = p.to_json();
        assert!(j.starts_with('{') && j.ends_with('}'));
        assert!(!j.chars().any(|c| (c as u32) < 0x20), "controle cru no JSON");
        assert_eq!(j.matches('{').count(), j.matches('}').count());
        assert!(p.resumo(Idioma::Pt).contains("Achados"));
        assert!(p.resumo(Idioma::En).contains("Findings"));
        let _ = margem_segura(&p).json().render();
    }
}

#[test]
fn json_escapa_nul_aspas_e_unicode() {
    let s = J::Str("a\u{0}\"\\\n\u{2028}é".into()).render();
    assert_eq!(s, "\"a\\u0000\\\"\\\\\\n\\u2028é\"");
    assert_eq!(J::Float(f64::NAN).render(), "null");
}

#[test]
fn parsers_hostis() {
    assert!(parse_limite("max\n", "x").unwrap() == Limite::Max);
    assert!(
        parse_limite("-1", "x").is_err()
            && parse_limite("1e9", "x").is_err()
            && parse_limite("", "x").is_err()
    );
    assert!(parse_meminfo("").is_err() && parse_meminfo("MemTotal: x kB").is_err());
    assert!(parse_psi("some avg10=1").is_err() && parse_psi("\u{0}").is_err());
    assert!(parse_cgroup_proprio("0::/a/../b").is_err() && parse_cgroup_proprio("0::rel").is_err());
    assert_eq!(parse_cgroup_proprio("0::/a/b (deleted)\n").unwrap(), "/a/b");
    assert_eq!(parse_algoritmo_zram("lzo [zstd] lz4"), Some("zstd".into()));
    assert_eq!(parse_algoritmo_zram("lzo [zstd"), None);
    assert_eq!(parse_swaps("h\na b 1 2 3\nx y z\n").len(), 1);
    assert_eq!(sanear("a\u{0}b\u{7}c ünï"), "abc ünï");
    assert_eq!(sanear(&"x".repeat(1000)).len(), 256);
    assert!(parse_uid("Uid:\tabc").is_err());
}

#[test]
fn niveis_e_efetivo() {
    assert_eq!(cgroup::niveis_acima("/a/b/c"), vec!["/a/b/c", "/a/b", "/a"]);
    assert!(cgroup::niveis_acima("/").is_empty());
    let n = |m: Option<Limite>, c: Option<u64>| NivelCgroup {
        memory_max: m,
        memory_current: c,
        ..Default::default()
    };
    let e = cgroup::efetivo(&[
        n(Some(Limite::Valor(100)), Some(40)),
        n(Some(Limite::Valor(50)), Some(10)),
        n(Some(Limite::Max), None),
    ]);
    assert_eq!((e.memory_max, e.folga_bytes), (Some(50), Some(40)));
}
