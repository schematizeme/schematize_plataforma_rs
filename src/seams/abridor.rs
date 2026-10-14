//! Abrir URL, pasta ou arquivo com o aplicativo padrão do sistema.
//!
//! **O quê:** o seam [`Abridor`] e a implementação [`AbridorUnix`] (`xdg-open` no
//! Linux/FreeBSD, `open` no macOS). Movido de `schematize_overdev_rs/src/gui/terminal.rs`
//! (`soltar`, `abrir_url`, `abrir_pasta`, linhas ~65-110).
//!
//! **Onde:** botões «abrir no navegador» / «abrir pasta» das janelas. O lançamento é
//! injetável (`abrir_com`) para os testes nunca abrirem navegador de verdade.

use std::path::Path;

/// **O quê:** lançador injetável `(programa, args)`. **Onde:** [`abrir_com`] e o `TerminalUnix`.
pub type Lancar<'a> = &'a dyn Fn(&str, &[String]) -> Result<(), String>;

/// **O quê:** chamadas registradas pelos testes. **Onde:** testes.
#[cfg(test)]
type Chamadas = Vec<(String, Vec<String>)>;

/// **O quê:** abre um alvo (URL, pasta ou arquivo) no aplicativo padrão.
///
/// **Onde:** injetado nas janelas; testes usam um falso.
pub trait Abridor: Send + Sync {
    /// **O quê:** abre `alvo` desacoplado do processo. **Onde:** ações de abrir link/pasta.
    fn abrir(&self, alvo: &str) -> Result<(), String>;
}

/// **O quê:** implementação real (`xdg-open` / `open`). **Onde:** composição do app.
#[derive(Debug, Clone, Copy, Default)]
pub struct AbridorUnix;

/// **O quê:** valida o alvo: vazio, só espaços, NUL ou `-` inicial (viraria flag do
/// `xdg-open`) são recusados. **Onde:** [`abrir_com`], antes de qualquer processo.
pub fn validar_alvo(alvo: &str) -> Result<(), String> {
    if alvo.trim().is_empty() {
        return Err("alvo vazio: não há o que abrir.".to_string());
    }
    if alvo.contains('\0') {
        return Err("alvo inválido: contém byte NUL.".to_string());
    }
    if alvo.starts_with('-') {
        return Err(format!("alvo inválido (começa com `-`, seria lido como opção): {alvo}"));
    }
    Ok(())
}

/// **O quê:** o programa que abre alvos neste SO (`open` no macOS, `xdg-open` nos demais).
/// Pura: o SO é argumento. **Onde:** [`AbridorUnix::abrir`] e os testes.
pub fn programa_para(os: &str) -> &'static str {
    if os == "macos" {
        "open"
    } else {
        "xdg-open"
    }
}

/// **O quê:** valida `alvo` e o entrega a `prog` via `lancar(prog, args)`. Só o `lancar`
/// toca o sistema. **Onde:** [`AbridorUnix::abrir`] e os testes com lançador falso.
pub fn abrir_com(
    prog: &str,
    alvo: &str,
    existe: &dyn Fn(&str) -> bool,
    lancar: Lancar,
) -> Result<(), String> {
    validar_alvo(alvo)?;
    if !existe(prog) {
        return Err(format!("não achei `{prog}` para abrir. Abra à mão: {alvo}"));
    }
    lancar(prog, &[alvo.to_string()])
        .map_err(|e| format!("não consegui rodar `{prog}` ({e}). Abra à mão: {alvo}"))
}

/// **O quê:** `true` se `bin` é arquivo executável em algum dir do `$PATH`. **Onde:**
/// [`AbridorUnix::abrir`] (e o `TerminalUnix` repete a ideia). Substitui o `command -v`.
pub fn no_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|d| eh_executavel(&d.join(bin)))
}

/// **O quê:** `p` é arquivo e (em unix) tem bit de execução. **Onde:** [`no_path`].
fn eh_executavel(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// **O quê:** spawn real desacoplado (stdio nulo, grupo de processos próprio).
/// **Onde:** [`AbridorUnix::abrir`]. Efeito: cria processo.
fn lancar_real(prog: &str, args: &[String]) -> Result<(), String> {
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

impl Abridor for AbridorUnix {
    /// **O quê:** abre `alvo` com o programa do SO. **Onde:** ações de abrir link/pasta.
    fn abrir(&self, alvo: &str) -> Result<(), String> {
        abrir_com(programa_para(std::env::consts::OS), alvo, &no_path, &lancar_real)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// **O quê:** roda `abrir_com` com falsos e devolve resultado + chamadas. **Onde:** testes.
    fn rodar(alvo: &str, tem: bool) -> (Result<(), String>, Chamadas) {
        let calls = Mutex::new(Vec::new());
        let r = abrir_com("xdg-open", alvo, &|_| tem, &|p, a| {
            calls.lock().unwrap().push((p.to_string(), a.to_vec()));
            Ok(())
        });
        (r, calls.into_inner().unwrap())
    }

    /// **O quê:** alvos sãos (https, espaço, unicode, aspas, `$()`) vão como UM argumento.
    /// **Onde:** teste.
    #[test]
    fn alvo_vai_como_um_argumento_sem_shell() {
        for a in [
            "https://x.dev/a?b=1&c=2",
            "/p/meu projeto",
            "/p/é ñ 日本",
            "/p/'\"",
            "/p/$(rm -rf ~)",
            "a\nb",
        ] {
            let (r, c) = rodar(a, true);
            assert!(r.is_ok(), "{a}");
            assert_eq!(c, vec![("xdg-open".to_string(), vec![a.to_string()])]);
        }
    }

    /// **O quê:** vazio, branco, NUL e `-` inicial são recusados sem lançar. **Onde:** teste.
    #[test]
    fn alvo_hostil_e_recusado_sem_lancar() {
        for a in ["", "   ", "\n", "a\0b", "-flag", "--help", "-"] {
            let (r, c) = rodar(a, true);
            assert!(r.is_err(), "{a:?}");
            assert!(c.is_empty(), "{a:?} não pode lançar");
        }
    }

    /// **O quê:** sem o programa o erro nomeia o programa e devolve o alvo. **Onde:** teste.
    #[test]
    fn sem_programa_o_erro_diz_o_que_fazer() {
        let (r, c) = rodar("https://x.dev", false);
        let e = r.unwrap_err();
        assert!(e.contains("xdg-open") && e.contains("https://x.dev"), "{e}");
        assert!(c.is_empty());
    }

    /// **O quê:** falha do spawn é propagada com o alvo, não engolida. **Onde:** teste.
    #[test]
    fn falha_do_spawn_e_propagada() {
        let r = abrir_com("xdg-open", "/p", &|_| true, &|_, _| Err("boom".to_string()));
        let e = r.unwrap_err();
        assert!(e.contains("boom") && e.contains("/p"), "{e}");
    }

    /// **O quê:** macOS usa `open`; linux/freebsd/windows `xdg-open`. **Onde:** teste.
    #[test]
    fn programa_por_so() {
        assert_eq!(programa_para("macos"), "open");
        assert_eq!(programa_para("linux"), "xdg-open");
        assert_eq!(programa_para("freebsd"), "xdg-open");
    }

    /// **O quê:** binário inexistente não está no PATH e `sh` está (unix). **Onde:** teste.
    #[test]
    fn no_path_distingue_existente_de_inexistente() {
        assert!(!no_path("binario-que-nao-existe-em-lugar-nenhum-42"));
        assert!(!no_path(""));
        #[cfg(unix)]
        assert!(no_path("sh"));
    }
}
