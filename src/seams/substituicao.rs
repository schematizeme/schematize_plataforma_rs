//! Seam de substituição de binário em disco (trocar executável em uso, chmod, matar GUI velha).
//!
//! **O quê:** o trait [`Substituicao`] e a impl Unix [`SubstituicaoUnix`], movidos de
//! `schematize_market_rs` (`atualizar/binario.rs` `substitui_binario`/`make_executable`,
//! `nucleo/util.rs` `definir_modo`, `atualizar/fonte.rs` `kill_stale_gui`).
//!
//! **Onde:** quem atualiza binários (market, deployer) recebe um `&dyn Substituicao`; testes
//! injetam o runner de `pkill` e o `sleep`, então nunca matam processo real.

use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

/// Tentativas e espera (ms) quando o SO diz `ETXTBSY` — mesmos números do market (1s em 25ms).
#[cfg(unix)]
const ESPERA_ETXTBSY: (u32, u64) = (40, 25);

/// `errno` de `ETXTBSY` ("Text file busy") no Linux/BSD.
#[cfg(unix)]
const ETXTBSY: i32 = 26;

/// **O quê:** contrato de troca de binário no disco.
///
/// **Onde:** consumido pelos atualizadores; implementado por [`SubstituicaoUnix`].
pub trait Substituicao: Send + Sync {
    /// **O quê:** coloca `novo` em `destino` sem matar quem executa o destino.
    ///
    /// **Onde:** fim de cada build/download de binário.
    fn substituir(&self, novo: &Path, destino: &Path) -> Result<(), String>;
    /// **O quê:** dá bit de execução (`0o755`) a `path`.
    ///
    /// **Onde:** todo binário recém-baixado ou copiado.
    fn tornar_executavel(&self, path: &Path) -> Result<(), String>;
    /// **O quê:** encerra a GUI velha pelo nome EXATO; falha vai para stderr, nunca engolida.
    ///
    /// **Onde:** depois de trocar o executável da GUI.
    fn encerrar_gui(&self, nome: &str);
}

/// **O quê:** impl Unix do [`Substituicao`] (rename atômico sobre o destino, `pkill -x`).
///
/// **Onde:** composição dos apps em Linux/macOS/BSD.
#[cfg(unix)]
#[derive(Debug, Default, Clone, Copy)]
pub struct SubstituicaoUnix;

/// **O quê:** cria o diretório-pai de `dst` (e ancestrais) se faltar.
///
/// **Onde:** [`substituir_com`] (máquina sem `~/.cargo/bin`, §37.48). Original: `garante_diretorio_pai`.
#[cfg(unix)]
fn garante_diretorio_pai(dst: &Path) -> Result<(), String> {
    match dst.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => std::fs::create_dir_all(dir)
            .map_err(|e| format!("não consegui criar o diretório {}: {e}", dir.display())),
        _ => Ok(()),
    }
}

/// **O quê:** `<destino>.novo` NO MESMO diretório (rename atômico exige o mesmo filesystem).
///
/// **Onde:** [`substituir_com`]. Original: `caminho_temporario`.
#[cfg(unix)]
fn caminho_temporario(dst: &Path) -> PathBuf {
    dst.with_file_name(format!(
        "{}.novo",
        dst.file_name().and_then(|s| s.to_str()).unwrap_or("schematize")
    ))
}

/// **O quê:** `true` se o erro é `ETXTBSY` (arquivo ainda aberto para escrita por um fork).
///
/// **Onde:** [`com_retentativa`].
#[cfg(unix)]
fn e_etxtbsy(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(ETXTBSY)
}

/// **O quê:** roda `op` retentando SÓ em `ETXTBSY` (`tentativas` x `ms`), dormindo via `dormir`.
///
/// **Onde:** cópia e rename de [`substituir_com`]. Outro erro é definitivo e volta na hora.
#[cfg(unix)]
fn com_retentativa(
    tentativas: u32,
    ms: u64,
    dormir: &dyn Fn(u64),
    op: &mut dyn FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut ultimo = None;
    for _ in 0..tentativas.max(1) {
        match op() {
            Ok(()) => return Ok(()),
            Err(e) if e_etxtbsy(&e) => {
                ultimo = Some(e);
                dormir(ms);
            }
            Err(e) => return Err(e),
        }
    }
    Err(ultimo.unwrap_or_else(|| std::io::Error::from_raw_os_error(ETXTBSY)))
}

/// **O quê:** grava `novo` como `<destino>.novo`, marca `0o755` e faz `rename` por cima de
/// `destino` (inode antigo segue vivo para quem já executa). Remove o `.novo` se falhar.
///
/// **Onde:** [`Substituicao::substituir`] da [`SubstituicaoUnix`]; `dormir` injetável.
/// Original: `substitui_binario` (market). Deriva da retentativa ETXTBSY do market.
#[cfg(unix)]
fn substituir_com(novo: &Path, destino: &Path, dormir: &dyn Fn(u64)) -> Result<(), String> {
    if !novo.is_file() {
        return Err(format!("origem inexistente ou não é arquivo: {}", novo.display()));
    }
    garante_diretorio_pai(destino)?;
    let tmp = caminho_temporario(destino);
    if let Err(e) = std::fs::remove_file(&tmp) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("não consegui limpar {}: {e}", tmp.display()));
        }
    }
    let (n, ms) = ESPERA_ETXTBSY;
    com_retentativa(n, ms, dormir, &mut || std::fs::copy(novo, &tmp).map(|_| ()))
        .map_err(|e| format!("não consegui gravar {}: {e}", tmp.display()))?;
    if let Err(e) = definir_modo(&tmp, 0o755) {
        return Err(limpa_tmp(&tmp, e));
    }
    if let Err(e) = com_retentativa(n, ms, dormir, &mut || std::fs::rename(&tmp, destino)) {
        return Err(limpa_tmp(&tmp, format!("não consegui substituir {}: {e}", destino.display())));
    }
    Ok(())
}

/// **O quê:** apaga o temporário órfão e devolve a mensagem (anexando falha da limpeza).
///
/// **Onde:** caminhos de erro de [`substituir_com`].
#[cfg(unix)]
fn limpa_tmp(tmp: &Path, msg: String) -> String {
    match std::fs::remove_file(tmp) {
        Ok(()) => msg,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => msg,
        Err(e) => format!("{msg} (e não consegui remover {}: {e})", tmp.display()),
    }
}

/// **O quê:** aplica o modo POSIX `modo` a `path`; erro propagado (o original ignorava).
///
/// **Onde:** [`substituir_com`] e [`Substituicao::tornar_executavel`]. Original: `definir_modo`.
#[cfg(unix)]
fn definir_modo(path: &Path, modo: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(modo))
        .map_err(|e| format!("não consegui dar modo {modo:o} a {}: {e}", path.display()))
}

/// **O quê:** valida o nome da GUI para `pkill -x`: não vazio, só `[A-Za-z0-9_-]`, sem `-` inicial.
///
/// **Onde:** [`SubstituicaoUnix::encerrar_gui_com`]. `pkill -x` trata o nome como REGEX
/// (`.`, `*`, `|` casariam outros processos) e `-foo` viraria opção; por isso é allowlist.
#[cfg(unix)]
fn validar_nome_gui(nome: &str) -> Result<(), String> {
    if nome.is_empty() {
        return Err("nome da GUI vazio".to_string());
    }
    if nome.starts_with('-') {
        return Err(format!("nome da GUI não pode começar com '-': {nome:?}"));
    }
    if !nome.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("nome da GUI com caractere inválido: {nome:?}"));
    }
    Ok(())
}

/// **O quê:** monta os argumentos do `pkill`: `-x -- <nome>` (o `--` blinda contra opção).
///
/// **Onde:** [`SubstituicaoUnix::encerrar_gui_com`].
#[cfg(unix)]
fn args_pkill(nome: &str) -> Vec<String> {
    vec!["-x".to_string(), "--".to_string(), nome.to_string()]
}

/// **O quê:** mensagem de stderr para uma falha de `encerrar_gui` (função pura).
///
/// **Onde:** [`Substituicao::encerrar_gui`] da [`SubstituicaoUnix`].
#[cfg(unix)]
fn mensagem_falha_gui(nome: &str, erro: &str) -> String {
    format!("aviso: não consegui encerrar a GUI {nome:?}: {erro}")
}

/// **O quê:** runner real: executa `pkill` e devolve o código de saída.
///
/// **Onde:** [`Substituicao::encerrar_gui`] (produção). `pkill` ausente vira `Err`.
#[cfg(unix)]
fn pkill_real(args: &[String]) -> Result<i32, String> {
    let st = std::process::Command::new("pkill")
        .args(args)
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("não consegui executar pkill: {e}"))?;
    st.code().ok_or_else(|| "pkill terminado por sinal".to_string())
}

#[cfg(unix)]
impl SubstituicaoUnix {
    /// **O quê:** `encerrar_gui` com runner injetável. Código 0 (matou) e 1 (nada rodando) são
    /// sucesso; outro código, nome inválido ou falha do runner é `Err`.
    ///
    /// **Onde:** [`Substituicao::encerrar_gui`] e os testes (sem matar processo real).
    pub fn encerrar_gui_com(
        &self,
        nome: &str,
        runner: &dyn Fn(&[String]) -> Result<i32, String>,
    ) -> Result<(), String> {
        validar_nome_gui(nome)?;
        match runner(&args_pkill(nome))? {
            0 | 1 => Ok(()),
            c => Err(format!("pkill saiu com código {c}")),
        }
    }
}

#[cfg(unix)]
impl Substituicao for SubstituicaoUnix {
    /// **O quê:** troca o binário em `destino` por `novo` (ver [`substituir_com`]).
    ///
    /// **Onde:** atualizadores de binário.
    fn substituir(&self, novo: &Path, destino: &Path) -> Result<(), String> {
        substituir_com(novo, destino, &|ms| {
            std::thread::sleep(std::time::Duration::from_millis(ms))
        })
    }

    /// **O quê:** `chmod 0o755` via API do SO (sem shell-out).
    ///
    /// **Onde:** binário recém-baixado.
    fn tornar_executavel(&self, path: &Path) -> Result<(), String> {
        definir_modo(path, 0o755)
    }

    /// **O quê:** `pkill -x` da GUI; falha é registrada em stderr (o contrato devolve `()`).
    ///
    /// **Onde:** depois de trocar o executável da GUI.
    fn encerrar_gui(&self, nome: &str) {
        if let Err(e) = self.encerrar_gui_com(nome, &pkill_real) {
            eprintln!("{}", mensagem_falha_gui(nome, &e));
        }
    }
}

#[cfg(all(test, unix))]
mod testes {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::os::unix::fs::PermissionsExt;

    /// **O quê:** diretório de teste único sob `target/testes` (nunca /tmp nem HOME).
    ///
    /// **Onde:** todos os testes deste módulo.
    fn dir(nome: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/testes/substituicao")
            .join(format!("{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("criar dir de teste");
        d
    }

    /// **O quê:** escreve `conteudo` em `p`. **Onde:** montagem dos testes.
    fn escreve(p: &Path, conteudo: &[u8]) {
        std::fs::write(p, conteudo).expect("escrever fixture");
    }

    /// **O quê:** `sleep` falso que só conta chamadas. **Onde:** testes de retentativa.
    fn sem_dormir(c: &Cell<u32>) -> impl Fn(u64) + '_ {
        move |_| c.set(c.get() + 1)
    }

    /// **O quê:** troca o conteúdo e deixa o destino executável, sem `.novo` órfão.
    ///
    /// **Onde:** caminho feliz de `substituir`.
    #[test]
    fn substitui_arquivo_e_deixa_executavel() {
        let d = dir("feliz");
        let (n, dst) = (d.join("n"), d.join("bin"));
        escreve(&n, b"novo");
        escreve(&dst, b"velho");
        SubstituicaoUnix.substituir(&n, &dst).expect("substituir");
        assert_eq!(std::fs::read(&dst).unwrap(), b"novo");
        let modo = std::fs::metadata(&dst).unwrap().permissions().mode();
        assert_eq!(modo & 0o777, 0o755);
        assert!(!caminho_temporario(&dst).exists());
        assert!(n.exists(), "a origem não é consumida");
    }

    /// **O quê:** cria destino inexistente em diretório inexistente.
    ///
    /// **Onde:** máquina sem `~/.cargo/bin`.
    #[test]
    fn cria_diretorio_pai_ausente() {
        let d = dir("pai");
        let n = d.join("n");
        escreve(&n, b"x");
        let dst = d.join("a/b/c/bin");
        SubstituicaoUnix.substituir(&n, &dst).expect("substituir");
        assert_eq!(std::fs::read(&dst).unwrap(), b"x");
    }

    /// **O quê:** origem ausente, ou diretório, vira `Err` sem tocar o destino.
    ///
    /// **Onde:** entrada hostil de `substituir`.
    #[test]
    fn origem_ausente_ou_diretorio_e_erro() {
        let d = dir("origem");
        let dst = d.join("bin");
        escreve(&dst, b"intacto");
        assert!(SubstituicaoUnix.substituir(&d.join("nao"), &dst).is_err());
        assert!(SubstituicaoUnix.substituir(&d, &dst).is_err());
        assert_eq!(std::fs::read(&dst).unwrap(), b"intacto");
    }

    /// **O quê:** destino sob um arquivo comum (pai não-diretório) é `Err`, não pânico.
    ///
    /// **Onde:** destino impossível.
    #[test]
    fn destino_com_pai_arquivo_e_erro() {
        let d = dir("paiarq");
        let (n, f) = (d.join("n"), d.join("arq"));
        escreve(&n, b"x");
        escreve(&f, b"y");
        assert!(SubstituicaoUnix.substituir(&n, &f.join("bin")).is_err());
    }

    /// **O quê:** destino que é diretório falha e não deixa `.novo` órfão.
    ///
    /// **Onde:** rename inválido limpa o temporário.
    #[test]
    fn destino_diretorio_limpa_temporario() {
        let d = dir("destdir");
        let n = d.join("n");
        escreve(&n, b"x");
        let dst = d.join("alvo");
        std::fs::create_dir(&dst).unwrap();
        assert!(SubstituicaoUnix.substituir(&n, &dst).is_err());
        assert!(!caminho_temporario(&dst).exists());
    }

    /// **O quê:** nomes unicode/com espaço funcionam; `.novo` velho é sobrescrito.
    ///
    /// **Onde:** nomes hostis de destino.
    #[test]
    fn nome_unicode_com_espaco_e_novo_velho() {
        let d = dir("unicode");
        let (n, dst) = (d.join("n"), d.join("bin ção é"));
        escreve(&n, b"u");
        escreve(&caminho_temporario(&dst), b"lixo");
        SubstituicaoUnix.substituir(&n, &dst).expect("substituir");
        assert_eq!(std::fs::read(&dst).unwrap(), b"u");
    }

    /// **O quê:** troca uma CÓPIA em execução de `sleep` sem `Text file busy`.
    ///
    /// **Onde:** o motivo de existir do rename atômico.
    #[test]
    fn substitui_binario_em_execucao() {
        let d = dir("emexec");
        let (dst, n) = (d.join("sleepcopia"), d.join("novo"));
        let Some(sleep) =
            ["/bin/sleep", "/usr/bin/sleep"].iter().map(Path::new).find(|p| p.is_file())
        else {
            return;
        };
        std::fs::copy(sleep, &dst).unwrap();
        definir_modo(&dst, 0o755).unwrap();
        let mut filho = std::process::Command::new(&dst).arg("30").spawn().expect("spawn");
        escreve(&n, b"#!/bin/sh\nexit 0\n");
        let r = SubstituicaoUnix.substituir(&n, &dst);
        filho.kill().unwrap();
        filho.wait().unwrap();
        r.expect("substituir em uso");
        assert_eq!(std::fs::read(&dst).unwrap(), b"#!/bin/sh\nexit 0\n");
    }

    /// **O quê:** `tornar_executavel` põe 0o755; arquivo inexistente é `Err`.
    ///
    /// **Onde:** chmod propagando erro.
    #[test]
    fn tornar_executavel_ok_e_erro() {
        let d = dir("chmod");
        let f = d.join("f");
        escreve(&f, b"x");
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        SubstituicaoUnix.tornar_executavel(&f).expect("chmod");
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(SubstituicaoUnix.tornar_executavel(&d.join("nao")).is_err());
    }

    /// **O quê:** retentativa: ETXTBSY 3x e depois sucesso dorme 3x; esgotar dá `Err` após 40.
    ///
    /// **Onde:** corrida de fork herdando fd.
    #[test]
    fn retentativa_etxtbsy() {
        let dormiu = Cell::new(0);
        let falhas = Cell::new(3);
        let r = com_retentativa(40, 25, &sem_dormir(&dormiu), &mut || {
            if falhas.get() > 0 {
                falhas.set(falhas.get() - 1);
                Err(std::io::Error::from_raw_os_error(ETXTBSY))
            } else {
                Ok(())
            }
        });
        assert!(r.is_ok());
        assert_eq!(dormiu.get(), 3);
        let dormiu = Cell::new(0);
        let r = com_retentativa(40, 25, &sem_dormir(&dormiu), &mut || {
            Err(std::io::Error::from_raw_os_error(ETXTBSY))
        });
        assert!(r.is_err());
        assert_eq!(dormiu.get(), 40);
    }

    /// **O quê:** outro erro (ENOENT) NÃO é retentado.
    ///
    /// **Onde:** erro definitivo volta na hora.
    #[test]
    fn outro_erro_nao_retenta() {
        let dormiu = Cell::new(0);
        let r = com_retentativa(40, 25, &sem_dormir(&dormiu), &mut || {
            Err(std::io::Error::from_raw_os_error(2))
        });
        assert!(r.is_err());
        assert_eq!(dormiu.get(), 0);
    }

    /// **O quê:** `tentativas == 0` ainda tenta uma vez. **Onde:** boundary.
    #[test]
    fn zero_tentativas_tenta_uma_vez() {
        let chamadas = Cell::new(0);
        let r = com_retentativa(0, 1, &|_| {}, &mut || {
            chamadas.set(chamadas.get() + 1);
            Ok(())
        });
        assert!(r.is_ok());
        assert_eq!(chamadas.get(), 1);
    }

    /// **O quê:** runner falso que grava os args e devolve `codigo`. **Onde:** testes de GUI.
    fn runner<'a>(
        visto: &'a RefCell<Vec<Vec<String>>>,
        codigo: Result<i32, String>,
    ) -> impl Fn(&[String]) -> Result<i32, String> + 'a {
        move |a| {
            visto.borrow_mut().push(a.to_vec());
            codigo.clone()
        }
    }

    /// **O quê:** nome válido chama `pkill -x -- nome`; códigos 0/1 ok, 2 e falha são `Err`.
    ///
    /// **Onde:** contrato de `encerrar_gui_com`.
    #[test]
    fn encerrar_gui_args_e_codigos() {
        let v = RefCell::new(vec![]);
        let s = SubstituicaoUnix;
        assert!(s.encerrar_gui_com("schematize-gui", &runner(&v, Ok(0))).is_ok());
        assert_eq!(v.borrow()[0], ["-x", "--", "schematize-gui"]);
        assert!(s.encerrar_gui_com("g", &runner(&v, Ok(1))).is_ok());
        assert!(s.encerrar_gui_com("g", &runner(&v, Ok(2))).is_err());
        assert!(s.encerrar_gui_com("g", &runner(&v, Err("sem pkill".into()))).is_err());
    }

    /// **O quê:** nomes hostis são recusados SEM chamar o runner.
    ///
    /// **Onde:** vazio, espaço, regex, `-` inicial, NUL, quebra de linha, unicode.
    #[test]
    fn encerrar_gui_nomes_hostis() {
        let v = RefCell::new(vec![]);
        for nome in [
            "", " ", "a b", ".*", "a|b", "^x$", "(x)", "[a]", "-f", "--help", "-x", "a\0b", "a\nb",
            "gúi", ".", "a.b", "*",
        ] {
            assert!(
                SubstituicaoUnix.encerrar_gui_com(nome, &runner(&v, Ok(0))).is_err(),
                "{nome:?} devia ser recusado"
            );
        }
        assert!(v.borrow().is_empty(), "runner nunca deve ser chamado");
    }

    /// **O quê:** a mensagem de falha cita o nome e o erro. **Onde:** stderr de `encerrar_gui`.
    #[test]
    fn mensagem_de_falha_cita_nome_e_erro() {
        let m = mensagem_falha_gui("x", "boom");
        assert!(m.contains("\"x\"") && m.contains("boom"));
    }

    /// **O quê:** o trait é object-safe e `Send + Sync`; `encerrar_gui` com nome inválido não
    /// executa nada nem entra em pânico. **Onde:** uso real via `&dyn`.
    #[test]
    fn trait_objeto_e_encerrar_gui_invalido_nao_panica() {
        let b: Box<dyn Substituicao> = Box::new(SubstituicaoUnix);
        b.encerrar_gui("-rf");
        b.encerrar_gui("");
    }

    /// **O quê:** `validar_nome_gui` aceita nomes típicos. **Onde:** allowlist.
    #[test]
    fn nomes_validos_passam() {
        for n in ["schematize", "schematize-market_gui", "A1", "x-"] {
            assert!(validar_nome_gui(n).is_ok(), "{n}");
        }
    }
}
