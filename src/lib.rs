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
//!
//! O módulo [`home`] (resolução de `HOME`) compila com qualquer uma das duas.

#[cfg(feature = "base")]
pub mod arquivo;
#[cfg(feature = "base")]
pub mod bin;
#[cfg(any(feature = "parasita", feature = "base"))]
pub mod home;
#[cfg(feature = "parasita")]
pub mod parasita;
#[cfg(feature = "base")]
pub mod procedencia;
