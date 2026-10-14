//! **seams** — as costuras de plataforma (traits) e as implementações Unix (linux/freebsd/macos).
//!
//! **O quê:** cada capacidade que difere por SO vira um trait; [`Plataforma`] reúne um
//! `Box<dyn ...>` de cada. O host recebe `&Plataforma` e não sabe qual SO é. A implementação
//! Windows mora no repo `schematize_windows_rs`.
//!
//! **Onde:** `schematize_market_rs` e `schematize_overdev_rs` (F4 troca os call sites).

pub mod abridor;
pub mod atalhos;
pub mod atalhos_render;
pub mod caminhos;
pub mod pathsetup;
pub mod processos;
pub mod rede;
pub mod shell;
pub mod substituicao;
pub mod terminal;

pub use abridor::Abridor;
pub use atalhos::Atalhos;
pub use atalhos_render::EntradaAtalho;
pub use caminhos::{Caminhos, CaminhosUnix};
pub use pathsetup::PathSetup;
pub use processos::Processos;
pub use rede::Rede;
pub use shell::Shell;
pub use substituicao::Substituicao;
pub use terminal::{ScriptTerminal, Terminal};

/// Todas as costuras, injetadas no host.
///
/// `#[non_exhaustive]`: costuras novas entram sem quebrar quem a monta — use [`Plataforma::nova`]
/// (e os `com_*` para trocar as costuras com default).
#[non_exhaustive]
pub struct Plataforma {
    /// Execução de comandos de shell.
    pub shell: Box<dyn Shell>,
    /// Rede (texto e download).
    pub rede: Box<dyn Rede>,
    /// Troca de binário em uso e encerramento da GUI.
    pub substituicao: Box<dyn Substituicao>,
    /// Atalhos de desktop (`.desktop` + ícones).
    pub atalhos: Box<dyn Atalhos>,
    /// Garantir/orientar o `PATH`.
    pub path: Box<dyn PathSetup>,
    /// Abrir um script num terminal.
    pub terminal: Box<dyn Terminal>,
    /// Abrir URL/arquivo com o app padrão.
    pub abridor: Box<dyn Abridor>,
    /// Consulta de processos e recursos.
    pub processos: Box<dyn Processos>,
    /// Separador de `PATH` e listas de diretórios (default: Unix).
    pub caminhos: Box<dyn Caminhos>,
}

impl Plataforma {
    /// **O quê:** monta a plataforma com as 8 costuras base; as novas (hoje `caminhos`)
    /// entram com o default Unix e se trocam com `com_*`. Existe porque a struct é
    /// `non_exhaustive` (literal não compila fora da lib).
    /// **Onde:** o `main` de cada SO (o Windows, em `schematize_windows_rs`).
    #[allow(clippy::too_many_arguments)]
    pub fn nova(
        shell: Box<dyn Shell>,
        rede: Box<dyn Rede>,
        substituicao: Box<dyn Substituicao>,
        atalhos: Box<dyn Atalhos>,
        path: Box<dyn PathSetup>,
        terminal: Box<dyn Terminal>,
        abridor: Box<dyn Abridor>,
        processos: Box<dyn Processos>,
    ) -> Self {
        Plataforma {
            shell,
            rede,
            substituicao,
            atalhos,
            path,
            terminal,
            abridor,
            processos,
            caminhos: Box::new(CaminhosUnix),
        }
    }

    /// **O quê:** troca a costura de caminhos. **Onde:** o `main` do Windows.
    pub fn com_caminhos(mut self, c: Box<dyn Caminhos>) -> Self {
        self.caminhos = c;
        self
    }
}

#[cfg(unix)]
impl Plataforma {
    /// **O quê:** a plataforma Unix (linux/freebsd/macos) com as implementações reais.
    ///
    /// **Onde:** o `main` do host em Unix. Só existe com `cfg(unix)`.
    pub fn unix() -> Self {
        Plataforma {
            shell: Box::new(shell::ShellUnix),
            rede: Box::new(rede::RedeUnix),
            substituicao: Box::new(substituicao::SubstituicaoUnix),
            atalhos: Box::new(atalhos::AtalhosUnix::default()),
            path: Box::new(pathsetup::PathSetupUnix),
            terminal: Box::new(terminal::TerminalUnix),
            abridor: Box::new(abridor::AbridorUnix),
            processos: Box::new(processos::ProcessosUnix),
            caminhos: Box::new(CaminhosUnix),
        }
    }
}

#[cfg(all(test, unix))]
mod testes {
    use super::*;

    /// **O quê:** `unix()` monta todas as costuras e elas respondem sem tocar rede/HOME.
    ///
    /// **Onde:** teste de `Plataforma::unix`.
    #[test]
    fn unix_monta_e_responde() {
        let p = Plataforma::unix();
        assert!(p.shell.existe("sh"));
        assert!(!p.shell.existe("binario-inexistente-42"));
        assert!(p.processos.vivo(std::process::id()));
        assert!(p.abridor.abrir("").is_err());
        assert_eq!(p.caminhos.separador_path(), ':');
    }
}
