//! Utilidades de sistema de arquivos: fsync de diretório, escrita atômica binária, flock.

use super::erro::{io, Erro};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::raw::c_int;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

extern "C" {
    /// `flock(2)` da libc (já linkada pela std).
    fn flock(fd: c_int, operation: c_int) -> c_int;
}
const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;

/// **O quê:** `fsync` do diretório, para tornar durável criação/remoção/rename de entradas.
///
/// **Onde:** depois de criar segmento, renomear snapshot, remover segmento compactado.
pub fn sincroniza_dir(dir: &Path) -> Result<(), Erro> {
    let f = File::open(dir).map_err(io(format!("abrir diretório {}", dir.display())))?;
    f.sync_all().map_err(io(format!("fsync do diretório {}", dir.display())))
}

/// **O quê:** grava `bytes` em `tmp` e faz `sync_all`.
///
/// **Onde:** `escreve_atomico_bin`.
fn grava_tmp(tmp: &Path, bytes: &[u8]) -> Result<(), Erro> {
    let ctx = || format!("gravar {}", tmp.display());
    let mut f = File::create(tmp).map_err(io(ctx()))?;
    f.write_all(bytes).map_err(io(ctx()))?;
    f.sync_all().map_err(io(format!("fsync de {}", tmp.display())))
}

/// **O quê:** nome de temporário único por chamada/processo ao lado de `alvo`.
///
/// **Onde:** `escreve_atomico_bin`.
pub fn nome_tmp(alvo: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nome = alvo.file_name().and_then(|s| s.to_str()).unwrap_or("alvo");
    alvo.with_file_name(format!(".{nome}.tmp-{}-{n}", std::process::id()))
}

/// **O quê:** escrita atômica binária: tmp + fsync + rename + fsync do diretório.
///
/// **Onde:** snapshot. Mesma ideia de `crate::arquivo::escreve_atomico`, mas para bytes e com
/// fsync do diretório (aquela é `&str` e vive atrás da feature `base`). Falha remove o tmp.
pub fn escreve_atomico_bin(alvo: &Path, bytes: &[u8]) -> Result<(), Erro> {
    let tmp = nome_tmp(alvo);
    let r = grava_tmp(&tmp, bytes).and_then(|()| {
        std::fs::rename(&tmp, alvo).map_err(io(format!("renomear para {}", alvo.display())))
    });
    if let Err(e) = r {
        return match std::fs::remove_file(&tmp) {
            Ok(()) => Err(e),
            Err(r) if r.kind() == std::io::ErrorKind::NotFound => Err(e),
            Err(r) => Err(Erro::Io {
                contexto: format!("{e}; e não removi {}", tmp.display()),
                fonte: r,
            }),
        };
    }
    match alvo.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => sincroniza_dir(dir),
        None => sincroniza_dir(Path::new(".")),
    }
}

/// **O quê:** lock exclusivo (flock) de um arquivo, liberado quando o valor é descartado ou o
/// processo morre (o kernel solta o flock mesmo em SIGKILL).
///
/// **Onde:** garante escritor único do journal e do estado do vigia.
#[derive(Debug)]
pub struct Trava {
    _arquivo: File,
}

/// **O quê:** vivo? Checa `/proc/<pid>`. **Onde:** detecção de lock órfão.
pub fn pid_vivo(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// **O quê:** lê o pid gravado no arquivo de lock. **Onde:** mensagem de erro e órfão.
fn pid_gravado(f: &mut File) -> Option<u32> {
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    s.trim().parse().ok()
}

impl Trava {
    /// **O quê:** adquire o flock não-bloqueante de `caminho`, gravando o pid próprio. Devolve a
    /// trava e se o lock anterior era órfão (pid gravado morto).
    ///
    /// **Onde:** `Journal::abrir`. Segundo escritor recebe `Erro::LockOcupado` com o pid do dono.
    pub fn adquirir(caminho: &Path) -> Result<(Trava, bool), Erro> {
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(caminho)
            .map_err(io(format!("abrir lock {}", caminho.display())))?;
        // SAFETY: fd válido enquanto `f` vive; flock não toca memória.
        let r = unsafe { flock(f.as_raw_fd(), LOCK_EX | LOCK_NB) };
        if r != 0 {
            let e = std::io::Error::last_os_error();
            return if e.kind() == std::io::ErrorKind::WouldBlock {
                let dir = caminho.parent().map(Path::to_path_buf).unwrap_or_default();
                Err(Erro::LockOcupado { dir, pid: pid_gravado(&mut f) })
            } else {
                Err(Erro::Io { contexto: format!("flock {}", caminho.display()), fonte: e })
            };
        }
        let anterior = pid_gravado(&mut f);
        let orfao = anterior.is_some_and(|p| p != std::process::id() && !pid_vivo(p));
        f.set_len(0).map_err(io("limpar lock"))?;
        f.seek(SeekFrom::Start(0)).map_err(io("reposicionar lock"))?;
        f.write_all(std::process::id().to_string().as_bytes()).map_err(io("gravar pid no lock"))?;
        Ok((Trava { _arquivo: f }, orfao))
    }
}
