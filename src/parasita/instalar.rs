//! Mecânica de instalação do parasita: download, validação, lock e `rename`.
//!
//! **O quê:** baixar com `curl`, validar `--version` (com teto de tempo), mover só se válido e
//! coordenar processos concorrentes por lock de arquivo.
//!
//! **Onde:** `parasita::garantir_em`. Tudo aqui recebe o que precisa por parâmetro, para os
//! testes nunca tocarem o HOME real nem a rede.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// Idade a partir da qual um lock é considerado órfão (processo morto no meio do download).
pub const LOCK_VELHO: Duration = Duration::from_secs(10 * 60);

/// Teto de espera padrão do `--version` do binário baixado.
pub const VERSAO_TIMEOUT: Duration = Duration::from_secs(10);

/// Assinatura do downloader injetável: `(url, destino) -> Result`.
pub type Baixador<'a> = &'a dyn Fn(&str, &Path) -> Result<(), String>;

/// Desfecho de pegar o lock.
#[derive(Debug, PartialEq, Eq)]
pub enum Lock {
    /// O lock é nosso.
    Meu,
    /// Outro processo cuida (lock com menos de [`LOCK_VELHO`]).
    Ocupado,
}

/// **O quê:** baixa `url` para `dst` com `<programa> -fsSL --max-time 30 -o dst url`.
///
/// **Onde:** [`baixar_com_curl`] (programa `curl`) e os testes (programa inexistente, para ver o
/// erro real de "curl indisponível" sem rede).
pub fn baixar_com_programa(programa: &str, url: &str, dst: &Path) -> Result<(), String> {
    let saida = Command::new(programa)
        .args(["-fsSL", "--max-time", "30", "-o"])
        .arg(dst)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("{programa} indisponível: {e}"))?;
    if saida.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&saida.stderr);
    Err(format!("download falhou ({}) em {url}: {}", saida.status, err.trim()))
}

/// **O quê:** o downloader real: `curl -fsSL --max-time 30` (curl existe no Linux, macOS,
/// FreeBSD e Windows 10+).
///
/// **Onde:** injetado por `garantir`. Os testes injetam outro.
pub fn baixar_com_curl(url: &str, dst: &Path) -> Result<(), String> {
    baixar_com_programa("curl", url, dst)
}

/// **O quê:** repete uma execução enquanto o kernel responde `ETXTBSY` ("Text file busy", 26).
///
/// **Onde:** o `spawn` de `--version` do binário recém-gravado, em [`validar`].
///
/// **Por quê:** com outra thread fazendo `fork` em paralelo, o filho herda por instantes o
/// descritor de escrita do arquivo que acabou de ser gravado e o `exec` falha com 26. É
/// transitório: só em unix e só nesse errno repete (até 40 vezes, backoff de 5 a 25 ms);
/// qualquer outro erro sai na hora, com a causa.
pub fn repetir_em_etxtbsy<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    const ETXTBSY: i32 = 26;
    let mut tentativa: u64 = 0;
    loop {
        match f() {
            Err(e) if cfg!(unix) && e.raw_os_error() == Some(ETXTBSY) && tentativa < 40 => {
                tentativa += 1;
                std::thread::sleep(Duration::from_millis(5 * tentativa.min(5)));
            }
            r => return r,
        }
    }
}

/// **O quê:** `chmod 0755` (unix); no-op nas outras plataformas.
///
/// **Onde:** [`validar`], antes de executar o binário baixado.
#[cfg(unix)]
fn marcar_executavel(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o755)).map_err(|e| format!("chmod: {e}"))
}

/// **O quê:** no-op (fora de unix não há bit de execução).
///
/// **Onde:** [`validar`].
#[cfg(not(unix))]
fn marcar_executavel(_: &Path) -> Result<(), String> {
    Ok(())
}

/// **O quê:** `chmod +x` e exige que `<bin> --version` saia 0 em até `teto`. O filho roda com
/// `SCHEMATIZE_SEM_MARKET=1` (sem recursão: o binário baixado também tem o parasita).
///
/// **Onde:** [`instalar_um`], antes do `rename`. Repete se o kernel responder `ETXTBSY`.
/// Estourou o teto: mata o processo, espera, e diz que não terminou.
pub fn validar(bin: &Path, teto: Duration) -> Result<(), String> {
    marcar_executavel(bin)?;
    let mut filho = repetir_em_etxtbsy(|| {
        Command::new(bin)
            .arg("--version")
            .env("SCHEMATIZE_SEM_MARKET", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    })
    .map_err(|e| format!("o binário baixado não executa: {e}"))?;
    let inicio = Instant::now();
    loop {
        match filho.try_wait() {
            Ok(Some(st)) if st.success() => return Ok(()),
            Ok(Some(st)) => return Err(format!("`--version` saiu com {st}")),
            Ok(None) if inicio.elapsed() >= teto => return Err(matar(&mut filho, teto)),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("esperar `--version`: {e}")),
        }
    }
}

/// **O quê:** mata o filho que estourou o teto e devolve a mensagem (inclui falha ao matar).
///
/// **Onde:** [`validar`].
fn matar(filho: &mut std::process::Child, teto: Duration) -> String {
    let mut msg = format!("`--version` não terminou em {} s", teto.as_secs_f32());
    if let Err(e) = filho.kill() {
        msg.push_str(&format!(" (e não consegui matá-lo: {e})"));
    }
    if let Err(e) = filho.wait() {
        msg.push_str(&format!(" (e não consegui esperá-lo: {e})"));
    }
    msg
}

/// **O quê:** o caminho temporário `.novo` ao lado de `dst`. No Windows termina em `.exe`
/// (`x.novo.exe`), porque o SO decide executabilidade pela extensão.
///
/// **Onde:** [`instalar_um`].
pub fn caminho_novo(dst: &Path, os: &str) -> PathBuf {
    let mut novo = dst.as_os_str().to_owned();
    if os == "windows" {
        let s = novo.to_string_lossy().into_owned();
        let base = s.strip_suffix(".exe").unwrap_or(&s);
        return PathBuf::from(format!("{base}.novo.exe"));
    }
    novo.push(".novo");
    PathBuf::from(novo)
}

/// **O quê:** baixa -> `chmod +x` -> `--version` -> `rename` de UM binário; qualquer falha
/// remove o `.novo` e deixa o destino intocado.
///
/// **Onde:** laço de `garantir_em`, uma vez por binário que falta.
pub fn instalar_um(
    url: &str,
    dst: &Path,
    os: &str,
    teto: Duration,
    baixar: Baixador<'_>,
) -> Result<(), String> {
    let novo = caminho_novo(dst, os);
    let r = baixar(url, &novo)
        .and_then(|()| validar(&novo, teto))
        .and_then(|()| fs::rename(&novo, dst).map_err(|e| format!("rename: {e}")));
    match r {
        Ok(()) => Ok(()),
        Err(causa) => match fs::remove_file(&novo) {
            Ok(()) => Err(causa),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(causa),
            Err(e) => Err(format!("{causa} (e não removi {}: {e})", novo.display())),
        },
    }
}

/// **O quê:** a idade do lock; indisponível ou do futuro (relógio torto) conta como órfão.
///
/// **Onde:** [`tomar_lock`].
fn idade_do_lock(lock: &Path) -> Result<Duration, std::io::Error> {
    let t = fs::metadata(lock).and_then(|m| m.modified())?;
    Ok(SystemTime::now().duration_since(t).unwrap_or(LOCK_VELHO))
}

/// **O quê:** toma o lock com `create_new` e grava o PID. `Meu` = é nosso; `Ocupado` = outro
/// processo (lock com menos de [`LOCK_VELHO`]) cuida; lock mais velho é órfão: remove e retoma.
///
/// **Onde:** `garantir_em`, antes de baixar qualquer coisa.
pub fn tomar_lock(lock: &Path) -> Result<Lock, String> {
    use std::io::ErrorKind::{AlreadyExists, NotFound};
    if let Some(dir) = lock.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("criar {}: {e}", dir.display()))?;
    }
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(lock) {
            Ok(mut f) => {
                f.write_all(std::process::id().to_string().as_bytes())
                    .map_err(|e| format!("escrever lock: {e}"))?;
                return Ok(Lock::Meu);
            }
            Err(e) if e.kind() == AlreadyExists => match idade_do_lock(lock) {
                Ok(i) if i < LOCK_VELHO => return Ok(Lock::Ocupado),
                Ok(_) => match fs::remove_file(lock) {
                    Ok(()) => {}
                    Err(e) if e.kind() == NotFound => {}
                    Err(e) => return Err(format!("remover lock órfão: {e}")),
                },
                Err(e) if e.kind() == NotFound => {}
                Err(e) => return Err(format!("ler lock: {e}")),
            },
            Err(e) => return Err(format!("lock {}: {e}", lock.display())),
        }
    }
    Ok(Lock::Ocupado)
}
