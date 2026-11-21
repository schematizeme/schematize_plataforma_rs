//! Fixture de cgroupfs/proc simulados em diretório temporário.
//!
//! **O quê:** [`FixtureDir`] cria um diretório único em `temp_dir()`, escreve arquivos por
//! caminho relativo e se apaga no `Drop`. Use com [`super::fonte::FonteDir::com_raiz`].
//!
//! **Onde:** testes (`tests/memoria_*.rs`) e quem quiser testar código sobre [`super::Fonte`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Diretório temporário removido ao sair de escopo.
#[derive(Debug)]
pub struct FixtureDir {
    raiz: PathBuf,
}

impl FixtureDir {
    /// **O quê:** cria `temp_dir()/schematize-<prefixo>-<pid>-<n>`. **Onde:** início de cada teste.
    pub fn nova(prefixo: &str) -> io::Result<Self> {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let raiz =
            std::env::temp_dir().join(format!("schematize-{prefixo}-{}-{n}", std::process::id()));
        fs::create_dir_all(&raiz)?;
        Ok(FixtureDir { raiz })
    }

    /// **O quê:** raiz da fixture. **Onde:** `FonteDir::com_raiz(f.raiz())`.
    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// **O quê:** escreve (substitui) `rel` com `conteudo`, criando diretórios. `rel` não pode
    /// escapar da raiz. **Onde:** montar/alterar o estado simulado entre amostras.
    pub fn escrever(&self, rel: &str, conteudo: &str) -> io::Result<()> {
        self.escrever_bytes(rel, conteudo.as_bytes())
    }

    /// **O quê:** como [`Self::escrever`], com bytes crus (UTF-8 inválido, `\0`, truncado).
    /// **Onde:** testes de arquivo hostil.
    pub fn escrever_bytes(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
        if rel.split('/').any(|c| c == "..") {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "rel com '..'"));
        }
        let p = self.raiz.join(rel.trim_start_matches('/'));
        if let Some(pai) = p.parent() {
            fs::create_dir_all(pai)?;
        }
        fs::write(p, bytes)
    }

    /// **O quê:** remove `rel` (simula arquivo que some, ex.: PSI desligado). **Onde:** testes.
    pub fn remover(&self, rel: &str) -> io::Result<()> {
        fs::remove_file(self.raiz.join(rel.trim_start_matches('/')))
    }
}

impl Drop for FixtureDir {
    /// **O quê:** apaga a árvore; falha de limpeza vai para stderr (nunca silenciosa).
    /// **Onde:** fim do teste.
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.raiz) {
            eprintln!("aviso: fixture {} não removida: {e}", self.raiz.display());
        }
    }
}
