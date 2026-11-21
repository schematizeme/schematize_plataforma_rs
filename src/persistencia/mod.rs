//! **Persistência durável externa ao processo** (resiliência absoluta do orquestrador).
//!
//! **O quê:** infraestrutura para que o overdev retome exatamente de onde parou depois de
//! SIGKILL/OOM/queda de energia, sem perder nem duplicar trabalho:
//!
//! - [`Journal`]: WAL append-only (`[len][crc32][seq][tipo][payload]`), segmentos rotacionados,
//!   fsync configurável ([`Durabilidade`]), escritor único por flock, recuperação que trunca a
//!   cauda torn.
//! - [`Snapshot`]: atômico (tmp + fsync + rename + fsync do dir), versionado, com `.prev`.
//! - [`Maquina`] / [`Estado`]: `estado = snapshot + replay`, eventos idempotentes por seq.
//! - [`vigia`]: watchdog separado (backoff + jitter + circuit breaker), e [`unit_systemd`].
//!
//! **Onde:** o orquestrador (overdev) pluga seu modelo em [`Estado`]; o `schematize-vigia`
//! (`[[bin]]` do integrador) chama [`vigia::rodar`]. Std-only; só Unix (flock, /proc).
//!
//! Escrita atômica de texto simples continua em `crate::arquivo`; aqui há a variante binária
//! com fsync do diretório (`fsutil`), porque `arquivo` vive atrás de outra feature.

pub mod aleatorio;
pub mod crc32;
pub mod erro;
mod fsutil;
pub mod journal;
pub mod maquina;
pub mod registro;
mod segmento;
pub mod snapshot;
mod unit;
pub mod vigia;

pub use aleatorio::Rng;
pub use erro::Erro;
pub use journal::{Config, Durabilidade, Journal, Relatorio};
pub use maquina::{Estado, Maquina};
pub use registro::Registro;
pub use snapshot::{Origem, Snapshot};
pub use unit::unit_systemd;
pub use vigia::{rodar, Desfecho, VigiaCfg};
