//! NÚCLEO — HOME do usuário e resolução de executáveis.
//!
//! **O quê:** [`home`] (`HOME` -> `USERPROFILE`), [`cargo_bin_dir`] (`~/.cargo/bin`) e
//! [`resolve_bin`] (caminho ABSOLUTO de um executável: `~/.cargo/bin` primeiro, depois o
//! `$PATH`, com `.exe`/`.cmd`/... no Windows).
//!
//! **Onde:** qualquer app que precise achar `ssh`, `claude`, `schematize-market`... mesmo
//! aberto pelo lançador do desktop (cujo `$PATH` não inclui `~/.cargo/bin`).
//!
//! Terminal, `xdg-open` e afins NÃO estão aqui (fora do escopo da v0.1).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub use crate::home::{home_de, home_do_ambiente};

/// **O quê:** o HOME do usuário (`HOME`, depois `USERPROFILE`); `None` se nenhum serve
/// (vazio, em branco ou relativo).
///
/// **Onde:** todo caminho de dados do app; o chamador decide o que fazer sem HOME (degradar,
/// nunca `panic` — piso 10).
pub fn home() -> Option<PathBuf> {
    home_do_ambiente()
}

/// **O quê:** `<home>/.cargo/bin`.
///
/// **Onde:** [`resolve_bin_em`] e o parasita. Pura sobre `home`.
pub fn cargo_bin_dir_de(home: &Path) -> PathBuf {
    home.join(".cargo").join("bin")
}

/// **O quê:** `~/.cargo/bin` do usuário atual, ou `None` sem HOME.
///
/// **Onde:** apps que instalam binários do ecossistema.
pub fn cargo_bin_dir() -> Option<PathBuf> {
    home().map(|h| cargo_bin_dir_de(&h))
}

/// **O quê:** a REGRA de quais nomes de arquivo um executável pode ter, com a plataforma como
/// PARÂMETRO em vez de `#[cfg]` (testável em qualquer SO).
///
/// **Onde:** [`resolve_bin_em`]. No Windows, sem extensão no pedido, tenta o nome nu e depois
/// `.exe`, `.cmd`, `.bat`, `.com`; com ponto no nome respeita o que o chamador pediu.
pub fn nomes_de_executavel_em(bin: &str, windows: bool) -> Vec<String> {
    if !windows || bin.contains('.') {
        return vec![bin.to_string()];
    }
    let mut v = vec![bin.to_string()];
    v.extend([".exe", ".cmd", ".bat", ".com"].iter().map(|ext| format!("{bin}{ext}")));
    v
}

/// **O quê:** `true` se `p` é um arquivo regular que pode ser executado (em unix, com algum
/// bit `x`; em outros SO, só ser arquivo).
///
/// **Onde:** [`resolve_bin_em`].
fn executavel(p: &Path) -> bool {
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

/// **O quê:** nome aceitável para busca: não vazio, sem separador de caminho nem byte nulo.
/// Nome com `/` ou `\` não é "um binário no PATH" — evita `join` escapar do diretório.
///
/// **Onde:** [`resolve_bin_em`].
fn nome_valido(bin: &str) -> bool {
    !bin.trim().is_empty() && !bin.contains(['/', '\\', '\0'])
}

/// **O quê:** o miolo puro de [`resolve_bin`]: procura `bin` em `<home>/.cargo/bin` e depois
/// em cada diretório de `path_var`, para `windows` dado. Devolve o 1º caminho absoluto que
/// existe e é executável; `None` caso contrário (nunca um palpite).
///
/// **Onde:** [`resolve_bin`] e os testes (HOME/PATH falsos, sem tocar o ambiente real).
pub fn resolve_bin_em(
    bin: &str,
    home: Option<&Path>,
    path_var: Option<&OsString>,
    windows: bool,
) -> Option<PathBuf> {
    if !nome_valido(bin) {
        return None;
    }
    let cargo = home.map(cargo_bin_dir_de);
    let do_path = path_var.map(|p| std::env::split_paths(p).collect::<Vec<_>>());
    let dirs = cargo.into_iter().chain(do_path.into_iter().flatten());
    for dir in dirs {
        // PATH com entrada vazia/relativa significaria "o cwd": não é lugar de binário confiável.
        if !dir.is_absolute() {
            continue;
        }
        for nome in nomes_de_executavel_em(bin, windows) {
            let p = dir.join(&nome);
            if executavel(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// **O quê:** caminho ABSOLUTO de `bin` — primeiro em `~/.cargo/bin`, depois no `$PATH`.
/// `None` se não achar em lugar nenhum.
///
/// **Onde:** quem precisa executar uma ferramenta por caminho absoluto (`ssh`, `claude`,
/// `schematize-market`).
pub fn resolve_bin(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    resolve_bin_em(bin, home().as_deref(), path.as_ref(), cfg!(windows))
}

/// **O quê:** `true` se `bin` resolve em algum lugar — checagem barata para dar erro claro
/// antes de tentar spawnar algo inexistente.
///
/// **Onde:** guardas de pré-condição dos apps.
pub fn binary_in_path(bin: &str) -> bool {
    resolve_bin(bin).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// **O quê:** pasta temporária única por teste.
    ///
    /// **Onde:** os testes deste módulo.
    fn tmp(nome: &str) -> PathBuf {
        let d = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/target/testes"))
            .join(format!("plat-bin-{}-{nome}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// **O quê:** cria um arquivo executável (unix: 0755) em `dir/nome`.
    ///
    /// **Onde:** os testes deste módulo.
    fn exec(dir: &Path, nome: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(nome);
        fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    /// A regra do Windows é verificável no Linux porque a plataforma é ARGUMENTO.
    #[test]
    fn nomes_de_executavel_cobre_pathext_no_windows() {
        assert_eq!(nomes_de_executavel_em("ssh", false), vec!["ssh"]);
        let w = nomes_de_executavel_em("ssh", true);
        assert!(w.contains(&"ssh.exe".to_string()), "{w:?}");
        assert_eq!(w[0], "ssh", "o nome nu vem primeiro");
        assert_eq!(nomes_de_executavel_em("ssh.exe", true), vec!["ssh.exe"]);
    }

    /// `resolve_bin` devolve caminho ABSOLUTO e existente — ou `None`, nunca um palpite.
    #[cfg(unix)]
    #[test]
    fn resolve_bin_da_absoluto_ou_nada() {
        let p = resolve_bin("sh").expect("`sh` existe em qualquer unix");
        assert!(p.is_absolute() && p.is_file(), "{}", p.display());
        assert!(resolve_bin("binario-que-nao-existe-em-lugar-nenhum-42").is_none());
    }

    /// `~/.cargo/bin` vence o PATH quando os dois têm o binário.
    #[cfg(unix)]
    #[test]
    fn cargo_bin_vem_antes_do_path() {
        let d = tmp("ordem");
        let home = d.join("h");
        let no_cargo = exec(&cargo_bin_dir_de(&home), "ferr");
        exec(&d.join("p"), "ferr");
        let path = std::env::join_paths([d.join("p")]).unwrap();
        let r = resolve_bin_em("ferr", Some(&home), Some(&path), false);
        assert_eq!(r, Some(no_cargo));
        // Sem HOME cai para o PATH.
        assert_eq!(resolve_bin_em("ferr", None, Some(&path), false), Some(d.join("p/ferr")));
    }

    /// Arquivo sem bit de execução não é binário (um shell também o pularia).
    #[cfg(unix)]
    #[test]
    fn arquivo_sem_permissao_de_execucao_e_ignorado() {
        let d = tmp("semx");
        fs::write(d.join("naoexec"), "x").unwrap();
        let path = std::env::join_paths([&d]).unwrap();
        assert_eq!(resolve_bin_em("naoexec", None, Some(&path), false), None);
        // Diretório com o nome também não conta.
        fs::create_dir_all(d.join("dirbin")).unwrap();
        assert_eq!(resolve_bin_em("dirbin", None, Some(&path), false), None);
    }

    /// Com `windows = true`, `ferr` acha `ferr.exe` (e o nome nu tem prioridade).
    #[test]
    fn windows_acha_o_exe_sem_a_extensao() {
        let d = tmp("win");
        fs::write(d.join("ferr.exe"), "x").unwrap();
        let path = std::env::join_paths([&d]).unwrap();
        // Em não-unix o `executavel` aceita qualquer arquivo; em unix, damos o bit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(d.join("ferr.exe"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(resolve_bin_em("ferr", None, Some(&path), true), Some(d.join("ferr.exe")));
        assert_eq!(resolve_bin_em("ferr", None, Some(&path), false), None);
    }

    /// Hostis: nome vazio, só espaços, com separador, `..`, byte nulo, unicode inexistente.
    #[test]
    fn nomes_hostis_nao_resolvem_nem_escapam_do_diretorio() {
        let d = tmp("hostil");
        exec(&d, "ok");
        let path = std::env::join_paths([&d]).unwrap();
        for ruim in ["", "   ", "../ok", "/bin/sh", "a/b", "a\\b", "ok\0", "日本語", ".."] {
            assert_eq!(resolve_bin_em(ruim, None, Some(&path), false), None, "{ruim:?}");
        }
    }

    /// PATH relativo/vazio (= "o cwd") é ignorado; PATH ausente não derruba.
    #[test]
    fn path_relativo_vazio_ou_ausente_e_ignorado() {
        let rel = OsString::from(":relativo:.");
        assert_eq!(resolve_bin_em("sh", None, Some(&rel), false), None);
        assert_eq!(resolve_bin_em("sh", None, None, false), None);
        assert_eq!(resolve_bin_em("sh", Some(Path::new("/nao/existe")), None, false), None);
    }

    /// `cargo_bin_dir_de` com HOME unicode e espaços.
    #[test]
    fn cargo_bin_dir_aceita_home_com_unicode_e_espacos() {
        let h = Path::new("/home/josé silva/日本");
        assert_eq!(cargo_bin_dir_de(h), PathBuf::from("/home/josé silva/日本/.cargo/bin"));
    }
}
