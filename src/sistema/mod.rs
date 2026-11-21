//! **sistema** — identifica a configuração de memória/OOM/recursos do host (systemd-oomd,
//! zram, PSI, cgroup v2, sysctl...). **Somente leitura, sem sudo, não muda nada.**
//!
//! **O quê:** [`identificar`] devolve um [`Perfil`] (serializável à mão com [`Perfil::to_json`],
//! texto em [`Perfil::resumo`]) com [`Achado`]s acionáveis e [`margem_segura`].
//! **Onde:** base do módulo `memoria` (orçamento adaptativo) e da `jaula`.
//!
//! Tudo passa pela trait [`Fonte`]: ausência de arquivo/comando vira campo `None` com o motivo
//! em [`Lido`], nunca panic. Testes usam [`FonteFalsa`].
//!
//! ```
//! use schematize_plataforma::sistema::{identificar_host, margem_segura, Idioma};
//! let p = identificar_host();
//! let _ = (p.to_json(), p.resumo(Idioma::Pt), margem_segura(&p));
//! ```

pub mod achados;
pub mod cgroup;
pub mod daemons;
pub mod erro;
pub mod fixtures;
pub mod fonte;
pub mod json;
pub mod kernel;
pub mod margem;
pub mod memoria;
pub mod parse;
pub mod resumo;
pub mod tipos;

pub use erro::ErroSistema;
pub use fonte::{Fonte, FonteFalsa, FonteReal};
pub use json::{ParaJson, J};
pub use margem::margem_segura;
pub use tipos::*;

/// O quê: coleta tudo de `fonte` e deriva os achados. Nunca entra em pânico.
/// Onde: ponto de entrada para testes (com [`FonteFalsa`]) e para [`identificar_host`].
pub fn identificar(fonte: &dyn Fonte) -> Perfil {
    let kernel = kernel::coletar(fonte);
    let mut p = Perfil {
        cgroup: cgroup::coletar(fonte, kernel.uid.get().copied()),
        memoria: memoria::coletar(fonte),
        daemons: daemons::coletar(fonte),
        processo: kernel::coletar_processo(fonte),
        kernel,
        achados: Vec::new(),
    };
    p.achados = achados::derivar(&p);
    p
}

/// O quê: [`identificar`] sobre o host real ([`FonteReal`]). Onde: apps e orquestrador.
pub fn identificar_host() -> Perfil {
    identificar(&FonteReal)
}

impl Perfil {
    /// O quê: JSON compacto do perfil inteiro (campos `Lido` com valor/fonte/motivo).
    /// Onde: logs estruturados, diagnóstico, GUIs.
    pub fn to_json(&self) -> String {
        ParaJson::json(self).render()
    }

    /// O quê: resumo textual no idioma pedido. Onde: CLIs/GUIs/logs.
    pub fn resumo(&self, idioma: Idioma) -> String {
        resumo::resumo(self, idioma)
    }

    /// O quê: atalho para [`margem_segura`]. Onde: orçamento de memória.
    pub fn margem_segura(&self) -> MargemMemoria {
        margem_segura(self)
    }
}
