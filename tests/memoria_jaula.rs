//! Integração com os tipos da jaula (só com `--features jaula,memoria`).
#![cfg(all(feature = "memoria", feature = "jaula"))]

use schematize_plataforma::jaula::{estimar, TipoExecucao, MEM_MAX_MIB, MEM_MIN_MIB};
use schematize_plataforma::memoria::{PedidoAdmissao, TETO_MAX_MIB, TETO_MIN_MIB};

#[test]
fn constantes_espelham_a_jaula_e_pedido_vem_do_orcamento() {
    assert_eq!((TETO_MIN_MIB, TETO_MAX_MIB), (MEM_MIN_MIB, MEM_MAX_MIB));
    let o = estimar(TipoExecucao::Build);
    assert_eq!(
        PedidoAdmissao::de_orcamento("b1", &o),
        PedidoAdmissao { id: "b1".into(), teto_mib: 2048 }
    );
}
