//! Teste no host real: só lê e confere a FORMA (nunca valores).
#![cfg(feature = "sistema")]

use schematize_plataforma::sistema::*;

#[test]
fn host_real_tem_forma_valida() {
    let p = identificar_host();
    let j = p.to_json();
    assert!(j.starts_with("{\"cgroup\":") && j.ends_with('}'));
    for chave in ["\"memoria\":", "\"daemons\":", "\"kernel\":", "\"processo\":", "\"achados\":"] {
        assert!(j.contains(chave), "faltou {chave}");
    }
    assert!(!j.chars().any(|c| (c as u32) < 0x20));
    assert!(p.kernel.cpus.map(|n| n >= 1).unwrap_or(true));
    for l in [&p.cgroup.v2.fonte, &p.memoria.meminfo.fonte] {
        assert!(!l.is_empty());
    }
    assert!(p.memoria.meminfo.valor.is_some() == p.memoria.meminfo.motivo.is_none());
    assert!(p
        .achados
        .iter()
        .all(|a| a.codigo.starts_with("SYS-") && !a.mensagem.is_empty() && !a.sugestao.is_empty()));
    assert!(!p.resumo(Idioma::Pt).is_empty() && !p.resumo(Idioma::En).is_empty());
    let m = margem_segura(&p);
    assert!(m.reservavel_mib <= m.disponivel_host_mib);
}

#[test]
fn fonte_real_comando_inexistente_e_timeout() {
    use std::time::Duration;
    let f = FonteReal;
    assert!(matches!(
        f.rodar_cmd("/nao/existe/xyz", &[], Duration::from_secs(1)),
        Err(ErroSistema::Ausente(_))
    ));
    assert!(matches!(
        f.rodar_cmd("sleep", &["5"], Duration::from_millis(100)),
        Err(ErroSistema::Timeout(_))
    ));
    assert!(matches!(f.ler_arquivo("/nao/existe/xyz"), Err(ErroSistema::Ausente(_))));
}
