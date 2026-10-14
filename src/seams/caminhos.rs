//! Costura de caminhos: separador de `PATH` e listas de diretórios por SO.
//!
//! **O quê:** o trait [`Caminhos`] e [`CaminhosUnix`], que guarda as listas que o overdev
//! (`extras_unix` em `agentrun/pathenv.rs`) e o lançador (`fallback_bin_dirs` em
//! `agentrun/lancador.rs`) montavam à mão, na MESMA ordem.
//!
//! **Onde:** montagem do `PATH` do agente/PTY e resolução de binário fora do `PATH`. Puro:
//! compila em qualquer alvo (o Windows tem a sua impl em `schematize_windows_rs`).

use std::path::{Path, PathBuf};

/// **O quê:** onde o SO guarda binários e como separa o `PATH`.
/// **Onde:** injetado via `Plataforma::caminhos`; testes usam um falso.
pub trait Caminhos: Send + Sync {
    /// **O quê:** o separador de entradas do `PATH` (`:` no Unix, `;` no Windows).
    /// **Onde:** montagem do `PATH` do processo filho.
    fn separador_path(&self) -> char;

    /// **O quê:** diretórios que se ACRESCENTAM ao `PATH` do agente (processo filho num PTY),
    /// em ordem de preferência.
    /// **Onde:** spawn do `claude` pelo overdev.
    fn extras_path_agente(&self, home: &Path) -> Vec<PathBuf>;

    /// **O quê:** diretórios de fallback para achar um binário fora do `PATH` (o desktop dá um
    /// `PATH` mínimo), em ordem de busca.
    /// **Onde:** `resolve_bin` do overdev/market.
    fn dirs_fallback_bin(&self, home: &Path) -> Vec<PathBuf>;
}

/// **O quê:** a implementação Unix (linux/freebsd/macos) — as listas de sempre.
/// **Onde:** `Plataforma::unix()` e o default de `Plataforma::nova`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CaminhosUnix;

impl Caminhos for CaminhosUnix {
    /// **O quê:** `':'`. **Onde:** montagem do `PATH`.
    fn separador_path(&self) -> char {
        ':'
    }

    /// **O quê:** `~/.local/bin`, `~/.cargo/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`.
    /// **Onde:** spawn do agente (ex-`extras_unix`).
    fn extras_path_agente(&self, home: &Path) -> Vec<PathBuf> {
        vec![
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ]
    }

    /// **O quê:** `~/.local/bin`, `~/.claude/local`, `~/.cargo/bin`, `/usr/local/bin`,
    /// `/usr/bin`, `/opt/homebrew/bin`, `/usr/local/opt/openssh/bin`.
    /// **Onde:** `resolve_bin` (ex-`fallback_bin_dirs`).
    fn dirs_fallback_bin(&self, home: &Path) -> Vec<PathBuf> {
        vec![
            home.join(".local/bin"),
            home.join(".claude/local"),
            home.join(".cargo/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/opt/openssh/bin"),
        ]
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    /// **O quê:** as listas Unix são as de sempre, na ordem. **Onde:** teste.
    #[test]
    fn listas_unix_na_ordem_original() {
        let c = CaminhosUnix;
        let h = Path::new("/h");
        assert_eq!(c.separador_path(), ':');
        let s = |v: Vec<PathBuf>| v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>();
        assert_eq!(
            s(c.extras_path_agente(h)),
            ["/h/.local/bin", "/h/.cargo/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        );
        assert_eq!(
            s(c.dirs_fallback_bin(h)),
            [
                "/h/.local/bin",
                "/h/.claude/local",
                "/h/.cargo/bin",
                "/usr/local/bin",
                "/usr/bin",
                "/opt/homebrew/bin",
                "/usr/local/opt/openssh/bin"
            ]
        );
    }
}
