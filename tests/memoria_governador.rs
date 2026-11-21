//! Governador (política pura) e Historico (recomendação de teto, serialização linha a linha).
#![cfg(feature = "memoria")]

use schematize_plataforma::memoria::{
    percentil95, recomendar_teto_mib, Decisao, EstadoHost, Governador, Historico, Motivo,
    PedidoAdmissao, Politica, UnidadeAtiva,
};

fn host(t: u64, disp: u64, psi: f64) -> EstadoHost {
    EstadoHost { agora_ms: t, mem_total_mib: 16_000, mem_disponivel_mib: disp, psi_some_avg10: psi }
}

fn un(id: &str, teto: u64, atual: u64, taxa: f64) -> UnidadeAtiva {
    UnidadeAtiva { id: id.into(), teto_mib: teto, atual_mib: atual, mib_por_s: taxa }
}

fn fila(n: usize, teto: u64) -> Vec<PedidoAdmissao> {
    (0..n).map(|i| PedidoAdmissao { id: format!("f{i}"), teto_mib: teto }).collect()
}

fn gov() -> Governador {
    Governador::novo(Politica::default())
}

#[test]
fn admite_so_o_que_cabe_com_reserva() {
    let mut g = gov();
    // 8000 - 1024 margem = 6976; ativa reserva 1000 (teto 1024, atual 24) => 5976 => 5 de 1024.
    let d = g.decidir(&host(0, 8000, 0.0), &[un("a", 1024, 24, 0.0)], &fila(10, 1024));
    assert_eq!(d, vec![Decisao::Admitir { quantas: 5 }]);
}

#[test]
fn admissao_respeita_fifo_e_paralelismo_maximo() {
    let mut g = Governador::novo(Politica { max_paralelo: 3, ..Politica::default() });
    let mut f = fila(1, 1024);
    f.push(PedidoAdmissao { id: "grande".into(), teto_mib: 9960 });
    f.extend(fila(3, 500));
    assert_eq!(
        g.decidir(&host(0, 12_000, 0.0), &[], &f),
        vec![Decisao::Admitir { quantas: 1 }],
        "para no 1o que não cabe"
    );
    let d = g.decidir(&host(1, 12_000, 0.0), &[], &fila(10, 500));
    assert_eq!(d, vec![Decisao::Admitir { quantas: 3 }], "teto de paralelismo");
}

#[test]
fn sem_margem_nada_entra() {
    let mut g = gov();
    let d = g.decidir(&host(0, 1000, 0.0), &[], &fila(2, 500));
    assert!(!d.iter().any(|x| matches!(x, Decisao::Admitir { .. })), "{d:?}");
}

#[test]
fn estrangula_com_histerese_e_libera_gradual() {
    let mut g = gov();
    let ativas: Vec<_> = (0..4).map(|i| un(&format!("u{i}"), 1024, 300, 0.0)).collect();
    let d = g.decidir(&host(0, 6000, 15.0), &ativas, &fila(2, 500));
    assert_eq!(d, vec![Decisao::Estrangular { paralelismo_max: 3 }], "reduz, não mata, não admite");
    let d = g.decidir(&host(1000, 6000, 7.0), &ativas, &fila(2, 500));
    assert_eq!(d, vec![Decisao::Manter], "7% está entre libera(4) e estrangula(10): segura");
    let d = g.decidir(&host(2000, 6000, 1.0), &ativas, &fila(2, 500));
    assert_eq!(d, vec![Decisao::Liberar]);
    assert_eq!(g.limite_paralelo(), 3);
    g.decidir(&host(3000, 6000, 1.0), &ativas, &[]);
    assert_eq!(g.limite_paralelo(), 4, "volta +1 por decisão");
}

#[test]
fn nao_libera_sem_125_porcento_da_margem() {
    let mut g = gov();
    g.decidir(&host(0, 6000, 20.0), &[un("a", 1024, 10, 0.0)], &[]);
    // 1100 >= margem mas < 1280 (125%): continua estrangulado.
    let d = g.decidir(&host(1, 1100, 0.0), &[un("a", 1024, 10, 0.0)], &[]);
    assert!(!d.contains(&Decisao::Liberar), "{d:?}");
}

#[test]
fn escada_encolher_parar_teto_com_graca() {
    let mut g = gov();
    let u = |atual| [un("a", 1000, atual, 0.0)];
    let d = g.decidir(&host(0, 9000, 0.0), &u(900), &[]);
    assert_eq!(d, vec![Decisao::PedirEncolher { id: "a".into(), motivo: Motivo::AcimaDoHigh }]);
    assert_eq!(
        g.decidir(&host(5000, 9000, 0.0), &u(900), &[]),
        vec![Decisao::Manter],
        "dentro da graça"
    );
    let d = g.decidir(&host(10_000, 9000, 0.0), &u(970), &[]);
    assert_eq!(d, vec![Decisao::PedirParar { id: "a".into(), motivo: Motivo::PedidoIgnorado }]);
    let d = g.decidir(&host(20_000, 9000, 0.0), &u(970), &[]);
    assert_eq!(d, vec![Decisao::DeixarTetoAgir { id: "a".into() }]);
    assert_eq!(
        g.decidir(&host(30_000, 9000, 0.0), &u(970), &[]),
        vec![Decisao::Manter],
        "só uma vez"
    );
}

#[test]
fn unidade_que_encolhe_rearma_abaixo_de_70() {
    let mut g = gov();
    g.decidir(&host(0, 9000, 0.0), &[un("a", 1000, 900, 0.0)], &[]);
    g.decidir(&host(1000, 9000, 0.0), &[un("a", 1000, 800, 0.0)], &[]);
    g.decidir(&host(2000, 9000, 0.0), &[un("a", 1000, 600, 0.0)], &[]);
    let d = g.decidir(&host(3000, 9000, 0.0), &[un("a", 1000, 900, 0.0)], &[]);
    assert!(
        matches!(d.as_slice(), [Decisao::PedirEncolher { .. }]),
        "pedido novo após rearme: {d:?}"
    );
}

#[test]
fn projecao_de_estouro_pede_encolher_antes_do_high() {
    let mut g = gov();
    // atual 500/1000, +30 MiB/s => 500/30 = 16,7 s <= 30 s.
    let d = g.decidir(&host(0, 9000, 0.0), &[un("a", 1000, 500, 30.0)], &[]);
    assert_eq!(d, vec![Decisao::PedirEncolher { id: "a".into(), motivo: Motivo::ProjecaoEstoura }]);
    let mut g = gov();
    let d = g.decidir(&host(0, 9000, 0.0), &[un("a", 1000, 500, 5.0)], &[]);
    assert_eq!(d, vec![Decisao::Manter], "100 s de folga: tranquilo");
}

#[test]
fn host_sem_margem_pede_a_maior_consumidora() {
    let mut g = gov();
    let ativas = [un("a", 1024, 200, 0.0), un("b", 1024, 700, 0.0), un("c", 1024, 300, 0.0)];
    let d = g.decidir(&host(0, 900, 0.0), &ativas, &[]);
    assert!(
        d.contains(&Decisao::PedirEncolher { id: "b".into(), motivo: Motivo::MaiorConsumidora }),
        "{d:?}"
    );
}

#[test]
fn decisao_e_deterministica() {
    let ativas = [un("z", 1024, 900, 10.0), un("a", 1024, 900, 10.0)];
    let mut a = gov();
    let mut b = gov();
    assert_eq!(
        a.decidir(&host(0, 700, 12.0), &ativas, &fila(3, 500)),
        b.decidir(&host(0, 700, 12.0), &ativas, &fila(3, 500))
    );
}

#[test]
fn recomendacao_p95_vezes_125_com_clamp() {
    let picos: Vec<u64> = (1..=20).map(|i| i * 50).collect(); // 50..1000
    assert_eq!(percentil95(&picos), Some(950));
    assert_eq!(recomendar_teto_mib(&picos, 16_000), Some(1188)); // 950*1.25=1187,5 -> 1188
    assert_eq!(recomendar_teto_mib(&[100], 16_000), Some(500), "piso 500");
    assert_eq!(recomendar_teto_mib(&[5000], 16_000), Some(2048), "teto 2048");
    assert_eq!(recomendar_teto_mib(&[1000], 800), Some(800), "nunca acima do host");
    assert_eq!(recomendar_teto_mib(&[], 16_000), None);
    assert_eq!(recomendar_teto_mib(&[u64::MAX], 16_000), Some(2048), "sem overflow");
    let g = gov();
    assert_eq!(g.max_host_mib(&host(0, 0, 0.0)), 16_000 - 1024);
}

#[test]
fn historico_serializa_e_relê() {
    let mut h = Historico::novo();
    for p in [300, 400, 350] {
        h.registrar("build", p).expect("registrar");
    }
    h.registrar("agente", 900).expect("registrar");
    let txt = h.serializar();
    assert_eq!(txt, "v1|agente|900\nv1|build|300\nv1|build|400\nv1|build|350\n");
    let c = Historico::de_linhas(&txt);
    assert!(c.rejeitadas.is_empty());
    assert_eq!(c.historico, h);
    assert_eq!(h.recomendar("build", 16_000), Some(500));
    assert_eq!(h.recomendar("nunca-visto", 16_000), None);
    assert_eq!(Historico::linha("x", 7).as_deref(), Ok("v1|x|7"));
}

#[test]
fn historico_hostil_e_limite_de_picos() {
    let txt =
        "# comentario\n\nv1|ok|10\nv2|x|1\nv1|sem-mib\nv1|x|abc\nv1||5\nv1|a|b|c\nlixo\nv1|ok|-3\n";
    let c = Historico::de_linhas(txt);
    assert_eq!(c.historico.picos("ok"), vec![10]);
    let linhas: Vec<usize> = c.rejeitadas.iter().map(|r| r.numero).collect();
    assert_eq!(linhas, vec![4, 5, 6, 7, 8, 9, 10], "toda linha ruim é reportada, nenhuma engolida");
    let mut h = Historico::novo();
    assert!(
        h.registrar("a|b", 1).is_err()
            && h.registrar("a\nb", 1).is_err()
            && h.registrar("  ", 1).is_err()
    );
    for i in 0..100 {
        h.registrar("t", i).expect("registrar");
    }
    let p = h.picos("t");
    assert_eq!((p.len(), p[0], p[63]), (64, 36, 99), "guarda só os 64 mais recentes");
}
