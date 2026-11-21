//! **schematize_plataforma** — infraestrutura de plataforma compartilhada do ecossistema.
//!
//! **O quê:** o que era copiado em vários repos (market parasita, procedência de build,
//! resolução de `HOME`/binários e escrita atômica), num lugar só. Std-only, **zero dependências**.
//! Sem domínio (piso 6): nada aqui sabe o que é skill, catálogo ou deploy.
//!
//! **Onde:** dependência git pinada por tag nos apps (ver o README).
//!
//! ## Features
//!
//! | feature | módulos | default |
//! |---|---|---|
//! | `parasita` | [`parasita`] | sim |
//! | `base` | [`procedencia`], [`bin`], [`arquivo`] | sim |
//! | `seams` | `seams` — traits de plataforma + impl Unix | não |
//! | `jaula` | `jaula` — teto duro de memória por execução (OOM confinado) | não |
//! | `sistema` | `sistema` — detecção só-leitura do ambiente de memória/OOM do host | não |
//! | `memoria` | `memoria` (implica `sistema`) — PSI, `memory.events`, governador de admissão | não |
//! | `persistencia` | `persistencia` — journal WAL, snapshot atômico, vigia e o bin `schematize-vigia` | não |
//!
//! O módulo [`home`] (resolução de `HOME`) compila com qualquer uma delas.

#[cfg(feature = "base")]
pub mod arquivo;
#[cfg(feature = "base")]
pub mod bin;
#[cfg(any(feature = "parasita", feature = "base", feature = "seams"))]
pub mod home;
#[cfg(feature = "jaula")]
pub mod jaula;
#[cfg(feature = "memoria")]
pub mod memoria;
#[cfg(feature = "parasita")]
pub mod parasita;
#[cfg(feature = "persistencia")]
pub mod persistencia;
#[cfg(feature = "base")]
pub mod procedencia;
#[cfg(feature = "seams")]
pub mod seams;
#[cfg(feature = "sistema")]
pub mod sistema;
