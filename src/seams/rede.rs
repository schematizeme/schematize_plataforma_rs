//! Costura de rede: GET de texto e download, por shell-out ao cliente HTTP nativo.
//!
//! **O quê:** o trait [`Rede`] (injetável nos testes) e [`RedeUnix`], que usa `curl` e, no
//! FreeBSD sem curl, o `fetch(1)` do base. Sem crate HTTP, por contrato (zero deps).
//!
//! **Onde:** apps que leem versão remota/baixam binário. Porte de
//! `schematize_market_rs/src/nucleo/rede.rs` (`Cliente`/`escolher_cliente`/`cliente` ~45-70,
//! `fetch_texto_args`/`fetch_download_args` ~80-92, `get_text`/`download` ~113-190), sem os
//! ramos Windows nem os helpers de API do GitHub.

use std::path::{Path, PathBuf};
use std::process::Command;

/// User-Agent das requisições (o GitHub responde 403 a requisição sem UA identificável).
const UA: &str = "User-Agent: schematize-market";

/// **O quê:** a costura de rede.
/// **Onde:** chamadores recebem `&dyn Rede`; os testes injetam um falso.
pub trait Rede: Send + Sync {
    /// **O quê:** corpo de `url` como texto; `None` se falhar (rede fora, HTTP >= 400).
    /// **Onde:** leitura de versão remota.
    fn texto(&self, url: &str) -> Option<String>;

    /// **O quê:** baixa `url` para `destino` (via arquivo temporário + rename).
    /// **Onde:** download de binário pré-compilado.
    fn baixar(&self, url: &str, destino: &Path) -> Result<(), String>;
}

/// Cliente HTTP de linha de comando. `Fetch` é o `fetch(1)` do FreeBSD base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cliente {
    Curl,
    Fetch,
}

/// Executor injetável: `(programa, argv) -> stdout`. Erro é a causa em texto.
pub type Executor<'a> = &'a dyn Fn(&str, &[String]) -> Result<String, String>;

/// **O quê:** escolhe o cliente a partir de fatos medidos. PURA. `Fetch` só no FreeBSD sem
/// curl. **Onde:** [`cliente`] e os testes.
pub fn escolher_cliente(freebsd: bool, tem_curl: bool) -> Cliente {
    if freebsd && !tem_curl {
        Cliente::Fetch
    } else {
        Cliente::Curl
    }
}

/// **O quê:** o cliente a usar NESTA máquina (SO compilado + `PATH`).
/// **Onde:** [`RedeUnix`].
pub fn cliente() -> Cliente {
    escolher_cliente(cfg!(target_os = "freebsd"), super::shell::existe_no_path("curl"))
}

/// **O quê:** programa e argv para ler `url` como texto (timeout 20s). PURA.
/// **Onde:** [`texto_com`] e os testes.
pub fn texto_args(cli: Cliente, url: &str) -> (&'static str, Vec<String>) {
    match cli {
        Cliente::Curl => ("curl", args(&["-fsSL", "-m", "20", "-H", UA, url])),
        Cliente::Fetch => ("fetch", args(&["-q", "-T", "20", "-o", "-", url])),
    }
}

/// **O quê:** programa e argv para gravar `url` em `dest`. PURA. No curl: falha em HTTP>=400
/// (`-f`), segue redirect, `--connect-timeout 20`.
/// **Onde:** [`baixar_com`] e os testes.
pub fn baixar_args(cli: Cliente, url: &str, dest: &str) -> (&'static str, Vec<String>) {
    match cli {
        Cliente::Curl => ("curl", args(&["-fSL", "--connect-timeout", "20", "-o", dest, url])),
        Cliente::Fetch => ("fetch", args(&["-o", dest, url])),
    }
}

/// **O quê:** converte `&[&str]` em argv owned. **Onde:** os construtores de argv acima.
fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// **O quê:** caminho do temporário ao lado de `destino` (`<nome>.part`). PURA.
/// **Onde:** [`baixar_com`].
pub fn caminho_parcial(destino: &Path) -> PathBuf {
    let mut nome = destino.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    nome.push(".part");
    destino.with_file_name(nome)
}

/// **O quê:** executor real: spawna `prog`, stdin nulo, devolve stdout (texto) ou a causa
/// (inclusive a stderr). **Onde:** [`RedeUnix`]; efeito colateral: spawn de processo.
pub fn executar_real(prog: &str, argv: &[String]) -> Result<String, String> {
    let out = Command::new(prog)
        .args(argv)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("não consegui executar `{prog}`: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "`{prog}` falhou ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// **O quê:** GET de texto com cliente e executor injetados; saída aparada, `None` em erro.
/// **Onde:** [`RedeUnix::texto`] e os testes.
pub fn texto_com(cli: Cliente, url: &str, exec: Executor) -> Option<String> {
    let (prog, argv) = texto_args(cli, url);
    exec(prog, &argv).ok().map(|s| s.trim().to_string())
}

/// **O quê:** download com cliente e executor injetados: grava em `<destino>.part` e faz
/// rename para `destino` só se o executor teve sucesso e o arquivo existe; em falha remove o
/// parcial (erro da remoção é anexado à mensagem, nunca engolido).
/// **Onde:** [`RedeUnix::baixar`] e os testes.
pub fn baixar_com(cli: Cliente, url: &str, destino: &Path, exec: Executor) -> Result<(), String> {
    let parcial = caminho_parcial(destino);
    let parcial_s =
        parcial.to_str().ok_or_else(|| format!("caminho não-UTF-8: {}", parcial.display()))?;
    let (prog, argv) = baixar_args(cli, url, parcial_s);
    let res = exec(prog, &argv).and_then(|_| {
        if parcial.is_file() {
            std::fs::rename(&parcial, destino)
                .map_err(|e| format!("não consegui gravar {}: {e}", destino.display()))
        } else {
            Err(format!("o download de {url} não gerou arquivo"))
        }
    });
    match res {
        Ok(()) => Ok(()),
        Err(e) if parcial.exists() => match std::fs::remove_file(&parcial) {
            Ok(()) => Err(e),
            Err(r) => Err(format!("{e} (e não consegui remover {}: {r})", parcial.display())),
        },
        Err(e) => Err(e),
    }
}

/// **O quê:** implementação real do [`Rede`]: `curl` (ou `fetch` no FreeBSD sem curl).
/// **Onde:** composição dos apps Linux/macOS/FreeBSD.
pub struct RedeUnix;

impl Rede for RedeUnix {
    /// **O quê:** GET de texto com o cliente desta máquina. **Onde:** quem usa `&dyn Rede`.
    fn texto(&self, url: &str) -> Option<String> {
        texto_com(cliente(), url, &executar_real)
    }

    /// **O quê:** download atômico (temp + rename). **Onde:** quem usa `&dyn Rede`.
    fn baixar(&self, url: &str, destino: &Path) -> Result<(), String> {
        baixar_com(cliente(), url, destino, &executar_real)
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::cell::RefCell;

    /// **O quê:** diretório temporário único e vazio. **Onde:** testes de download.
    fn tmp(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seams_rede_{nome}_{}", std::process::id()));
        if d.exists() {
            std::fs::remove_dir_all(&d).expect("limpar");
        }
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    /// **O quê:** a tabela de escolha do cliente. **Onde:** `cargo test`.
    #[test]
    fn escolha_do_cliente() {
        assert_eq!(escolher_cliente(true, false), Cliente::Fetch);
        assert_eq!(escolher_cliente(true, true), Cliente::Curl);
        assert_eq!(escolher_cliente(false, false), Cliente::Curl);
        assert_eq!(escolher_cliente(false, true), Cliente::Curl);
    }

    /// **O quê:** argv exato de texto para curl e fetch. **Onde:** teste.
    #[test]
    fn argv_de_texto_exato() {
        let (p, a) = texto_args(Cliente::Curl, "https://x/y");
        assert_eq!(p, "curl");
        assert_eq!(a, ["-fsSL", "-m", "20", "-H", "User-Agent: schematize-market", "https://x/y"]);
        let (p, a) = texto_args(Cliente::Fetch, "https://x/y");
        assert_eq!(p, "fetch");
        assert_eq!(a, ["-q", "-T", "20", "-o", "-", "https://x/y"]);
    }

    /// **O quê:** argv exato de download para curl e fetch. **Onde:** teste.
    #[test]
    fn argv_de_download_exato() {
        let (p, a) = baixar_args(Cliente::Curl, "https://x/y", "/d/f.part");
        assert_eq!(p, "curl");
        assert_eq!(a, ["-fSL", "--connect-timeout", "20", "-o", "/d/f.part", "https://x/y"]);
        let (p, a) = baixar_args(Cliente::Fetch, "https://x/y", "/d/f.part");
        assert_eq!(p, "fetch");
        assert_eq!(a, ["-o", "/d/f.part", "https://x/y"]);
    }

    /// **O quê:** URL hostil vai como UM argumento (sem shell, sem injeção). **Onde:** teste.
    #[test]
    fn url_hostil_e_um_unico_argumento() {
        let url = "https://x/a b;rm -rf /$(id)`x`\n--output=/etc/passwd";
        let (_, a) = texto_args(Cliente::Curl, url);
        assert_eq!(a.last().map(String::as_str), Some(url));
        assert_eq!(a.len(), 6);
    }

    /// **O quê:** o parcial fica ao lado do destino, mesmo sem nome de arquivo. **Onde:** teste.
    #[test]
    fn parcial_ao_lado() {
        assert_eq!(caminho_parcial(Path::new("/a/b/f.bin")), Path::new("/a/b/f.bin.part"));
        assert_eq!(caminho_parcial(Path::new("f")), Path::new("f.part"));
        assert_eq!(caminho_parcial(Path::new("/")), Path::new("/.part"));
    }

    /// **O quê:** `texto_com` apara a saída, repassa o argv e mapeia erro para `None`.
    /// **Onde:** teste (sem rede).
    #[test]
    fn texto_com_executor_falso() {
        let vistos = RefCell::new(Vec::new());
        let ok = |p: &str, a: &[String]| {
            vistos.borrow_mut().push((p.to_string(), a.to_vec()));
            Ok("  v1.2.3\n".to_string())
        };
        assert_eq!(texto_com(Cliente::Curl, "https://u", &ok), Some("v1.2.3".into()));
        assert_eq!(vistos.borrow()[0].0, "curl");
        assert_eq!(vistos.borrow()[0].1.last().map(String::as_str), Some("https://u"));
        let falha = |_: &str, _: &[String]| Err("404".to_string());
        assert_eq!(texto_com(Cliente::Fetch, "https://u", &falha), None);
        let vazio = |_: &str, _: &[String]| Ok(String::new());
        assert_eq!(texto_com(Cliente::Curl, "", &vazio), Some(String::new()));
    }

    /// **O quê:** sucesso grava no destino via rename e não deixa `.part`. **Onde:** teste.
    #[test]
    fn baixar_sucesso_renomeia() {
        let d = tmp("ok");
        let destino = d.join("bin");
        let exec = |_: &str, a: &[String]| {
            let i = a.iter().position(|x| x == "-o").expect("-o") + 1;
            std::fs::write(&a[i], "dados").map_err(|e| e.to_string())?;
            Ok(String::new())
        };
        baixar_com(Cliente::Curl, "https://u", &destino, &exec).expect("baixar");
        assert_eq!(std::fs::read_to_string(&destino).expect("ler"), "dados");
        assert!(!d.join("bin.part").exists());
        std::fs::remove_dir_all(&d).expect("rm");
    }

    /// **O quê:** falha do executor preserva o destino antigo e remove o parcial. **Onde:** teste.
    #[test]
    fn baixar_falha_preserva_destino_e_limpa_parcial() {
        let d = tmp("falha");
        let destino = d.join("bin");
        std::fs::write(&destino, "antigo").expect("seed");
        let exec = |_: &str, a: &[String]| {
            let i = a.iter().position(|x| x == "-o").expect("-o") + 1;
            std::fs::write(&a[i], "metade").map_err(|e| e.to_string())?;
            Err("curl falhou (exit 22)".to_string())
        };
        let e = baixar_com(Cliente::Curl, "https://u", &destino, &exec).expect_err("deve falhar");
        assert!(e.contains("exit 22"), "{e}");
        assert_eq!(std::fs::read_to_string(&destino).expect("ler"), "antigo");
        assert!(!d.join("bin.part").exists());
        std::fs::remove_dir_all(&d).expect("rm");
    }

    /// **O quê:** executor "ok" que não gera arquivo é erro, nunca sucesso mudo. **Onde:** teste.
    #[test]
    fn baixar_sem_arquivo_e_erro() {
        let d = tmp("semarq");
        let destino = d.join("bin");
        let exec = |_: &str, _: &[String]| Ok(String::new());
        let e = baixar_com(Cliente::Fetch, "https://u", &destino, &exec).expect_err("erro");
        assert!(e.contains("não gerou arquivo"), "{e}");
        assert!(!destino.exists());
        std::fs::remove_dir_all(&d).expect("rm");
    }

    /// **O quê:** destino em diretório inexistente vira `Err` (rename falha) e limpa o parcial.
    /// **Onde:** teste.
    #[test]
    fn baixar_destino_em_dir_inexistente() {
        let d = tmp("semdir");
        let destino = d.join("nao/existe/bin");
        let exec = |_: &str, _: &[String]| Ok(String::new());
        assert!(baixar_com(Cliente::Curl, "u", &destino, &exec).is_err());
        std::fs::remove_dir_all(&d).expect("rm");
    }

    /// **O quê:** o executor real devolve stdout, e `Err` com a causa para status não-zero ou
    /// programa ausente. **Onde:** teste (spawna `sh`, sem rede).
    #[cfg(unix)]
    #[test]
    fn executor_real() {
        let ok = executar_real("sh", &args(&["-c", "printf oi"])).expect("ok");
        assert_eq!(ok, "oi");
        let e = executar_real("sh", &args(&["-c", "echo causa >&2; exit 4"])).expect_err("erro");
        assert!(e.contains("causa"), "{e}");
        let e = executar_real("schematize_inexistente_zzz", &[]).expect_err("erro");
        assert!(e.contains("não consegui executar"), "{e}");
    }
}
