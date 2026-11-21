//! Detecção do backend de confinamento disponível nesta máquina.
//!
//! **O quê:** [`Backend`] e [`detectar_backend`]; a decisão é a função pura [`decidir`].
//! **Onde:** [`super::executar`] consulta antes de rodar qualquer coisa.

use std::path::{Path, PathBuf};

/// Mecanismo que impõe o teto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `systemd-run --user --scope` com cgroup v2 e controller `memory` delegado.
    SystemdScope,
    /// `prlimit --as=` (RLIMIT_AS): teto por processo, sem cgroup.
    Prlimit,
    /// Nada disponível: a execução NÃO roda.
    Nenhum,
}

/// **O quê:** decisão pura do backend a partir de fatos já coletados.
/// **Onde:** [`detectar_backend`] (fatos reais) e os testes (fixtures).
pub fn decidir(
    tem_systemd_run: bool,
    tem_bus_usuario: bool,
    controllers_delegados: Option<&str>,
    tem_prlimit: bool,
) -> Backend {
    let memory = controllers_delegados.is_some_and(|c| c.split_whitespace().any(|x| x == "memory"));
    if tem_systemd_run && tem_bus_usuario && memory {
        Backend::SystemdScope
    } else if tem_prlimit {
        Backend::Prlimit
    } else {
        Backend::Nenhum
    }
}

/// **O quê:** `true` se `bin` é um executável no `PATH`.
/// **Onde:** [`detectar_backend`].
fn no_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|d| d.join(bin).is_file())
}

/// **O quê:** uid real lido de `/proc/self/status` (sem libc). **Onde:** [`detectar_backend`].
fn uid_atual() -> Option<u32> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let linha = s.lines().find(|l| l.starts_with("Uid:"))?;
    linha.split_whitespace().nth(1)?.parse().ok()
}

/// **O quê:** caminho do `cgroup.controllers` do manager do usuário (`user@UID.service`).
/// **Onde:** [`detectar_backend`].
fn controllers_do_usuario(uid: u32) -> PathBuf {
    Path::new("/sys/fs/cgroup/user.slice")
        .join(format!("user-{uid}.slice/user@{uid}.service/cgroup.controllers"))
}

/// **O quê:** sonda a máquina (systemd-run, bus de usuário, controllers delegados, prlimit).
/// **Onde:** início de [`super::executar`]; testes de integração para decidir se pulam.
pub fn detectar_backend() -> Backend {
    let controllers =
        uid_atual().and_then(|u| std::fs::read_to_string(controllers_do_usuario(u)).ok());
    let bus = std::env::var_os("XDG_RUNTIME_DIR").is_some();
    decidir(no_path("systemd-run"), bus, controllers.as_deref(), no_path("prlimit"))
}

/// **O quê:** mensagem para `Veredito::SemBackend`: acionável, sem culpa.
/// **Onde:** [`super::executar`] quando o backend é `Nenhum`.
pub fn motivo_sem_backend() -> String {
    "não encontrei como limitar a memória aqui, então não rodei para proteger o resto do \
     sistema. Eu consigo seguir se houver `systemd-run` com sessão de usuário e o controller \
     `memory` delegado (cgroup v2), ou o `prlimit` (util-linux) instalado."
        .to_string()
}
