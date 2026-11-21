//! Fonte de leitura de arquivos do kernel (`/proc`, `/sys`) por trait, para simular em teste.
//!
//! **O quê:** [`Fonte`] abstrai "ler um arquivo de texto"; [`FonteDir`] lê de verdade sob uma
//! raiz (`/` em produção, um tempdir em fixture). Toda leitura é limitada a
//! [`LIMITE_BYTES`] (arquivo hostil/gigante não estoura a memória do monitor).
//!
//! **Onde:** [`super::leitura::Leitura`] e [`super::observador::Observador`] só falam com
//! `&dyn Fonte`; nunca tocam o filesystem direto.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

/// Máximo de bytes lidos por arquivo (256 KiB): `io.stat` com muitos discos é o maior caso real.
pub const LIMITE_BYTES: u64 = 256 * 1024;

/// Falha ao ler um arquivo, com o caminho e a causa (nunca engolida: vira `motivo` do campo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErroLeitura {
    /// Caminho pedido.
    pub caminho: String,
    /// Causa legível.
    pub causa: String,
}

impl fmt::Display for ErroLeitura {
    /// **O quê:** `caminho: causa`. **Onde:** `motivo` de [`super::leitura::Campo`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.caminho, self.causa)
    }
}

impl std::error::Error for ErroLeitura {}

/// Lê arquivos de texto do kernel. `Send + Sync` para o thread do monitor.
pub trait Fonte: Send + Sync {
    /// **O quê:** conteúdo (UTF-8 lossy, até [`LIMITE_BYTES`]) do arquivo em `caminho` absoluto.
    /// **Onde:** toda leitura de cgroup/proc do módulo.
    fn ler(&self, caminho: &str) -> Result<String, ErroLeitura>;
}

/// [`Fonte`] sobre o filesystem, sob uma raiz.
#[derive(Debug, Clone)]
pub struct FonteDir {
    raiz: PathBuf,
}

impl FonteDir {
    /// **O quê:** fonte real (raiz `/`). **Onde:** produção.
    pub fn real() -> Self {
        FonteDir { raiz: PathBuf::from("/") }
    }

    /// **O quê:** fonte sob `raiz` (cgroupfs/proc simulados). **Onde:** testes com fixture.
    pub fn com_raiz(raiz: impl Into<PathBuf>) -> Self {
        FonteDir { raiz: raiz.into() }
    }
}

impl Fonte for FonteDir {
    /// **O quê:** abre `raiz + caminho` (recusa `..`), lê até o limite. **Onde:** ver [`Fonte`].
    fn ler(&self, caminho: &str) -> Result<String, ErroLeitura> {
        let erro = |causa: String| ErroLeitura { caminho: caminho.to_string(), causa };
        if caminho.split('/').any(|c| c == "..") {
            return Err(erro("componente '..' recusado".to_string()));
        }
        let p = self.raiz.join(caminho.trim_start_matches('/'));
        let f = File::open(&p).map_err(|e| erro(format!("abrir: {e}")))?;
        let mut buf = Vec::new();
        f.take(LIMITE_BYTES).read_to_end(&mut buf).map_err(|e| erro(format!("ler: {e}")))?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}
