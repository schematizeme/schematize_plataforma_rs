//! Seam de processos e recursos da máquina: quem está vivo, quantos, onde, quanta RAM e carga.
//!
//! **O quê:** o trait [`Processos`] e a implementação real [`ProcessosUnix`]. Linux lê `/proc`
//! (lógica movida de `overdev_rs`: `agents::count_claude_processes`/`mem_available_mb`/`loadavg1`,
//! `supervisor::agente_vivo_em`/`supervisor_vivo_em`, `trava::vivo`). FreeBSD, macOS e demais
//! unix usam `ps`/`lsof`/`procstat`/`sysctl`/`vm_stat`; Windows usa `tasklist`. Nunca "assume
//! vivo": o que não dá para saber vira `None`/`false`.
//!
//! **Onde:** quem precisa de pid vivo (trava), contagem de agentes e supervisor. O SO e os
//! executores são injetáveis ([`Sistema`], [`ProcFs`], [`Executor`]) para os testes rodarem os
//! parsers de TODOS os SOs com texto de fixture, sem tocar o `/proc` nem o `ps` reais.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[path = "processos_parse.rs"]
mod parse;
#[cfg(test)]
#[path = "processos_testes.rs"]
mod testes;

use parse::{
    args_de_cmdline, cmdline_casa, parse_carga, parse_freebsd_mb, parse_lsof_cwd, parse_meminfo_mb,
    parse_procstat_cwd, parse_ps, parse_tasklist, parse_vm_stat_mb, pid_em_ps,
};

/// Maior pid aceito (limite do `pid_t` com sinal); acima disso `kill`/`ps` não conhecem.
const PID_MAX: u32 = i32::MAX as u32;
/// Quantos pids vão por invocação de `lsof`/`procstat` (limita o tamanho da linha de comando).
const LOTE_PIDS: usize = 256;

/// Contrato de consulta a processos e recursos. `None` = desconhecido (nunca um número falso).
pub trait Processos: Send + Sync {
    /// **O quê:** o processo `pid` existe? `pid` 0 e acima de `i32::MAX` são sempre `false`.
    ///
    /// **Onde:** checagem de dono de trava (`trava::orfa`).
    fn vivo(&self, pid: u32) -> bool;
    /// **O quê:** quantos processos têm o comando `nome` (argv0, ou runtime rodando `nome`).
    ///
    /// **Onde:** orçamento de agentes (`agents::count_claude_processes`).
    fn contar(&self, nome: &str) -> usize;
    /// **O quê:** algum processo `nome` tem cwd exatamente em `dir`?
    ///
    /// **Onde:** supervisor do overdev (`agente_vivo_em`, `supervisor_vivo_em`).
    fn em_dir(&self, nome: &str, dir: &Path) -> bool;
    /// **O quê:** RAM disponível em MB, ou `None` se não der para saber.
    ///
    /// **Onde:** orçamento de agentes (`agents::mem_available_mb`).
    fn memoria_disponivel_mb(&self) -> Option<u64>;
    /// **O quê:** load average de 1 minuto, ou `None` se não der para saber.
    ///
    /// **Onde:** orçamento de agentes (`agents::loadavg1`).
    fn carga1(&self) -> Option<f64>;
}

/// SO alvo dos comandos/parsers; injetável para o Linux testar os três parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sistema {
    /// `/proc`.
    Linux,
    /// `ps`, `procstat`, `sysctl`.
    FreeBsd,
    /// `ps`, `lsof`, `vm_stat`, `sysctl`.
    MacOs,
    /// `tasklist`.
    Windows,
    /// Outro unix: POSIX `ps` + `lsof` + `sysctl`, se existirem.
    Outro,
}

impl Sistema {
    /// **O quê:** o SO em que o binário está rodando (`cfg!(target_os)`).
    ///
    /// **Onde:** [`ProcessosCom::real`].
    pub fn atual() -> Self {
        if cfg!(target_os = "linux") {
            Sistema::Linux
        } else if cfg!(target_os = "freebsd") {
            Sistema::FreeBsd
        } else if cfg!(target_os = "macos") {
            Sistema::MacOs
        } else if cfg!(windows) {
            Sistema::Windows
        } else {
            Sistema::Outro
        }
    }
}

/// Acesso somente-leitura ao `/proc` (injetável nos testes).
pub trait ProcFs: Send + Sync {
    /// **O quê:** pids listados em `/proc`; `None` se o diretório não abre.
    ///
    /// **Onde:** `contar`/`em_dir` no Linux.
    fn pids(&self) -> Option<Vec<u32>>;
    /// **O quê:** `/proc/<pid>` existe?
    ///
    /// **Onde:** `vivo` no Linux.
    fn existe(&self, pid: u32) -> bool;
    /// **O quê:** bytes de `/proc/<pid>/cmdline`; `None` se sumiu ou sem permissão.
    ///
    /// **Onde:** `contar`/`em_dir` no Linux.
    fn cmdline(&self, pid: u32) -> Option<Vec<u8>>;
    /// **O quê:** destino de `/proc/<pid>/cwd`; `None` se sumiu ou sem permissão.
    ///
    /// **Onde:** `em_dir` no Linux.
    fn cwd(&self, pid: u32) -> Option<PathBuf>;
    /// **O quê:** texto de `/proc/<arquivo>` (`meminfo`, `loadavg`); `None` se ilegível/não-UTF-8.
    ///
    /// **Onde:** `memoria_disponivel_mb`/`carga1` no Linux.
    fn texto(&self, arquivo: &str) -> Option<String>;
}

/// Executor de comandos externos (injetável nos testes).
pub trait Executor: Send + Sync {
    /// **O quê:** roda `prog args...` e devolve o stdout (qualquer status de saída: `ps -p`
    /// sai 1 quando o pid não existe). `None` só se o programa nem executou.
    ///
    /// **Onde:** `ProcessosCom` fora do Linux.
    fn rodar(&self, prog: &str, args: &[&str]) -> Option<String>;
}

/// `/proc` real.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcReal;

impl ProcFs for ProcReal {
    /// **O quê:** lista `/proc`, ficando só com entradas numéricas.
    ///
    /// **Onde:** trait [`ProcFs`].
    fn pids(&self) -> Option<Vec<u32>> {
        let dir = std::fs::read_dir("/proc").ok()?;
        Some(dir.flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).collect())
    }
    /// **O quê:** `/proc/<pid>` existe (movido de `trava::vivo`).
    ///
    /// **Onde:** trait [`ProcFs`].
    fn existe(&self, pid: u32) -> bool {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    /// **O quê:** lê o cmdline cru.
    ///
    /// **Onde:** trait [`ProcFs`].
    fn cmdline(&self, pid: u32) -> Option<Vec<u8>> {
        std::fs::read(format!("/proc/{pid}/cmdline")).ok()
    }
    /// **O quê:** resolve o link `cwd`.
    ///
    /// **Onde:** trait [`ProcFs`].
    fn cwd(&self, pid: u32) -> Option<PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    /// **O quê:** lê um arquivo de texto de `/proc`.
    ///
    /// **Onde:** trait [`ProcFs`].
    fn texto(&self, arquivo: &str) -> Option<String> {
        std::fs::read_to_string(Path::new("/proc").join(arquivo)).ok()
    }
}

/// Executor real via `std::process::Command`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutorReal;

impl Executor for ExecutorReal {
    /// **O quê:** executa e captura o stdout (UTF-8 com perda).
    ///
    /// **Onde:** trait [`Executor`].
    fn rodar(&self, prog: &str, args: &[&str]) -> Option<String> {
        let saida = Command::new(prog).args(args).output().ok()?;
        Some(String::from_utf8_lossy(&saida.stdout).into_owned())
    }
}

/// Implementação parametrizável (SO + `/proc` + executor): é o motor de [`ProcessosUnix`].
#[derive(Clone)]
pub struct ProcessosCom {
    sistema: Sistema,
    proc: Arc<dyn ProcFs>,
    cmd: Arc<dyn Executor>,
}

impl ProcessosCom {
    /// **O quê:** motor com o SO, o `/proc` e o executor reais.
    ///
    /// **Onde:** [`ProcessosUnix`].
    pub fn real() -> Self {
        Self::novo(Sistema::atual(), Arc::new(ProcReal), Arc::new(ExecutorReal))
    }

    /// **O quê:** motor com tudo injetado.
    ///
    /// **Onde:** testes (e quem quiser forçar um SO).
    pub fn novo(sistema: Sistema, proc: Arc<dyn ProcFs>, cmd: Arc<dyn Executor>) -> Self {
        Self { sistema, proc, cmd }
    }

    /// **O quê:** `ps -axo pid=,comm=,args=` parseado; `None` se o `ps` não executou.
    ///
    /// **Onde:** `contar`/`em_dir` fora do Linux/Windows.
    fn lista_ps(&self) -> Option<Vec<(u32, Vec<String>)>> {
        Some(parse_ps(&self.cmd.rodar("ps", &["-axo", "pid=,comm=,args="])?))
    }

    /// **O quê:** pids dos processos `nome`, pelo `/proc` (Linux) ou pelo `ps` (demais unix).
    ///
    /// **Onde:** `contar` e `em_dir`.
    fn pids_de(&self, nome: &str) -> Vec<u32> {
        if nome.is_empty() {
            return Vec::new();
        }
        if self.sistema == Sistema::Linux {
            let pids = self.proc.pids().unwrap_or_default();
            return pids
                .into_iter()
                .filter(|p| {
                    let raw = self.proc.cmdline(*p).unwrap_or_default();
                    cmdline_casa(&args_de_cmdline(&raw), nome)
                })
                .collect();
        }
        let lista = self.lista_ps().unwrap_or_default();
        lista.into_iter().filter(|(_, a)| cmdline_casa(a, nome)).map(|(p, _)| p).collect()
    }

    /// **O quê:** algum dos `pids` tem cwd == `dir`, perguntando a `procstat` (FreeBSD) ou
    /// `lsof` (macOS/outros) em lotes.
    ///
    /// **Onde:** `em_dir` fora do Linux.
    fn cwd_por_comando(&self, pids: &[u32], dir: &Path) -> bool {
        pids.chunks(LOTE_PIDS).any(|lote| {
            let ids: Vec<String> = lote.iter().map(u32::to_string).collect();
            let cwds = if self.sistema == Sistema::FreeBsd {
                let mut a = vec!["-f"];
                a.extend(ids.iter().map(String::as_str));
                self.cmd.rodar("procstat", &a).map(|t| parse_procstat_cwd(&t))
            } else {
                let lista = ids.join(",");
                let a = ["-a", "-d", "cwd", "-Fn", "-p", lista.as_str()];
                self.cmd.rodar("lsof", &a).map(|t| parse_lsof_cwd(&t))
            };
            cwds.unwrap_or_default().iter().any(|c| c.as_path() == dir)
        })
    }
}

impl Processos for ProcessosCom {
    /// **O quê:** vivo por `/proc/<pid>` (Linux), `tasklist` (Windows) ou `ps -p` (unix).
    ///
    /// **Onde:** trait [`Processos`].
    fn vivo(&self, pid: u32) -> bool {
        if pid == 0 || pid > PID_MAX {
            return false;
        }
        match self.sistema {
            Sistema::Linux => self.proc.existe(pid),
            Sistema::Windows => {
                let filtro = format!("PID eq {pid}");
                let a = ["/FI", filtro.as_str(), "/NH", "/FO", "CSV"];
                let saida = self.cmd.rodar("tasklist", &a).unwrap_or_default();
                parse_tasklist(&saida).iter().any(|(_, p)| *p == pid)
            }
            _ => {
                let id = pid.to_string();
                let saida = self.cmd.rodar("ps", &["-p", id.as_str(), "-o", "pid="]);
                pid_em_ps(&saida.unwrap_or_default(), pid)
            }
        }
    }

    /// **O quê:** conta os processos de comando `nome` (heurística de `cmdline_e_claude`).
    ///
    /// **Onde:** trait [`Processos`].
    fn contar(&self, nome: &str) -> usize {
        if self.sistema == Sistema::Windows {
            let alvo = nome.trim_end_matches(".exe").to_lowercase();
            if alvo.is_empty() {
                return 0;
            }
            let saida = self.cmd.rodar("tasklist", &["/NH", "/FO", "CSV"]).unwrap_or_default();
            let n = |s: &str| s.to_lowercase().trim_end_matches(".exe").to_string();
            return parse_tasklist(&saida).iter().filter(|(i, _)| n(i) == alvo).count();
        }
        self.pids_de(nome).len()
    }

    /// **O quê:** algum processo `nome` com cwd == `dir`.
    ///
    /// **Onde:** trait [`Processos`].
    fn em_dir(&self, nome: &str, dir: &Path) -> bool {
        if self.sistema == Sistema::Windows {
            return false;
        }
        let pids = self.pids_de(nome);
        if self.sistema == Sistema::Linux {
            return pids.iter().any(|p| self.proc.cwd(*p).is_some_and(|c| c == dir));
        }
        self.cwd_por_comando(&pids, dir)
    }

    /// **O quê:** RAM disponível por `/proc/meminfo`, `sysctl` (FreeBSD) ou `vm_stat` (macOS).
    ///
    /// **Onde:** trait [`Processos`].
    fn memoria_disponivel_mb(&self) -> Option<u64> {
        match self.sistema {
            Sistema::Linux => parse_meminfo_mb(&self.proc.texto("meminfo")?),
            Sistema::FreeBsd => {
                let a = [
                    "-n",
                    "hw.pagesize",
                    "vm.stats.vm.v_free_count",
                    "vm.stats.vm.v_inactive_count",
                ];
                parse_freebsd_mb(&self.cmd.rodar("sysctl", &a)?)
            }
            Sistema::MacOs => parse_vm_stat_mb(&self.cmd.rodar("vm_stat", &[])?),
            Sistema::Windows | Sistema::Outro => None,
        }
    }

    /// **O quê:** load de 1 min por `/proc/loadavg` ou `sysctl -n vm.loadavg`.
    ///
    /// **Onde:** trait [`Processos`].
    fn carga1(&self) -> Option<f64> {
        match self.sistema {
            Sistema::Linux => parse_carga(&self.proc.texto("loadavg")?),
            Sistema::Windows => None,
            _ => parse_carga(&self.cmd.rodar("sysctl", &["-n", "vm.loadavg"])?),
        }
    }
}

/// Implementação real para o SO em que o binário roda (Linux/FreeBSD/macOS/Windows/outro).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessosUnix;

impl Processos for ProcessosUnix {
    /// **O quê:** delega ao motor real.
    ///
    /// **Onde:** trait [`Processos`].
    fn vivo(&self, pid: u32) -> bool {
        ProcessosCom::real().vivo(pid)
    }
    /// **O quê:** delega ao motor real.
    ///
    /// **Onde:** trait [`Processos`].
    fn contar(&self, nome: &str) -> usize {
        ProcessosCom::real().contar(nome)
    }
    /// **O quê:** delega ao motor real.
    ///
    /// **Onde:** trait [`Processos`].
    fn em_dir(&self, nome: &str, dir: &Path) -> bool {
        ProcessosCom::real().em_dir(nome, dir)
    }
    /// **O quê:** delega ao motor real.
    ///
    /// **Onde:** trait [`Processos`].
    fn memoria_disponivel_mb(&self) -> Option<u64> {
        ProcessosCom::real().memoria_disponivel_mb()
    }
    /// **O quê:** delega ao motor real.
    ///
    /// **Onde:** trait [`Processos`].
    fn carga1(&self) -> Option<f64> {
        ProcessosCom::real().carga1()
    }
}
