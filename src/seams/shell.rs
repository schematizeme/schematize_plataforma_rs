//! Costura de shell: roda comando via login shell e sonda binário no `PATH`.
//!
//! **O quê:** o trait [`Shell`] (injetável nos testes) e [`ShellUnix`], a implementação real
//! que executa `<shell> -lc <cmd>` com a saída capturada.
//!
//! **Onde:** apps que precisam rodar receitas/instaladores de shell. Porte de
//! `schematize_market_rs/src/nucleo/util.rs` (`login_shell`, linhas ~160-190) e da busca no
//! `PATH` de `nucleo/bin.rs` (`binary_in_path`).

use std::io;
use std::path::Path;
use std::process::{ExitStatus, Output, Stdio};

/// **O quê:** a costura de shell — executa um comando e diz se um binário existe.
/// **Onde:** chamadores recebem `&dyn Shell`; os testes injetam um falso.
pub trait Shell: Send + Sync {
    /// **O quê:** roda `cmd` no shell de login e devolve a saída capturada.
    /// **Onde:** receitas de instalação/sondagem dos apps.
    fn run(&self, cmd: &str) -> io::Result<Output>;

    /// **O quê:** `true` se `bin` resolve no `PATH` como executável.
    /// **Onde:** sondas "a ferramenta X está instalada?".
    fn existe(&self, bin: &str) -> bool;

    /// **O quê:** executa `argv` DIRETO (sem shell de login, sem parsing de shell). Com
    /// `herdar_stdio` o filho herda stdin/stdout/stderr (prompt do `sudo`, progresso ao vivo);
    /// sem ele, saída capturada e descartada e stdin nulo. `argv` vazio é `InvalidInput`.
    /// **Onde:** receitas que precisam de um comando exato com o terminal da pessoa.
    /// Método com default (v0.2.1, não quebra implementadores): usa `std::process::Command`.
    fn executar(&self, argv: &[&str], herdar_stdio: bool) -> io::Result<ExitStatus> {
        let (prog, args) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "argv vazio"))?;
        let mut c = std::process::Command::new(prog);
        c.args(args);
        if herdar_stdio {
            c.stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        } else {
            c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        }
        c.status()
    }

    /// **O quê:** roda `cmd` no shell de login HERDANDO o terminal (saída ao vivo, `sudo`
    /// consegue pedir a senha). **Onde:** execução de passos de instalação já consentidos.
    /// Default (v0.2.1): `sh -c` herdado; o Unix sobrescreve com `<login_shell> -lc`.
    fn run_herdado(&self, cmd: &str) -> io::Result<ExitStatus> {
        self.executar(&["sh", "-c", cmd], true)
    }
}

/// **O quê:** escolhe o shell a partir de fatos já medidos. PURA. `sh` só no FreeBSD sem bash
/// (o base não traz bash); em qualquer outro caso `bash`.
/// **Onde:** [`login_shell`] e os testes.
pub fn escolher_shell(is_freebsd: bool, bash_present: bool) -> &'static str {
    if is_freebsd && !bash_present {
        "sh"
    } else {
        "bash"
    }
}

/// **O quê:** o shell a usar NESTA máquina (mede o SO compilado e o `PATH`).
/// **Onde:** [`ShellUnix::run`].
pub fn login_shell() -> &'static str {
    escolher_shell(cfg!(target_os = "freebsd"), existe_no_path("bash"))
}

/// **O quê:** `true` se `p` é um arquivo regular com algum bit de execução (Unix); fora do
/// Unix basta ser arquivo. **Onde:** [`existe_em`].
fn e_executavel(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// **O quê:** procura `bin` nos diretórios de `path` (formato do `PATH`). Nome vazio ou com
/// separador de caminho nunca resolve por busca (evita `../x`); entradas vazias do `PATH` são
/// ignoradas (não viram o cwd). PURA em relação ao ambiente.
/// **Onde:** [`existe_no_path`] e os testes.
pub fn existe_em(bin: &str, path: &std::ffi::OsStr) -> bool {
    if bin.is_empty() || bin.contains('/') || bin.contains('\\') || bin.contains('\0') {
        return false;
    }
    std::env::split_paths(path)
        .filter(|d| !d.as_os_str().is_empty())
        .any(|d| e_executavel(&d.join(bin)))
}

/// **O quê:** [`existe_em`] sobre o `PATH` real do processo. `false` se `PATH` não existe.
/// **Onde:** [`login_shell`], [`ShellUnix::existe`] e `rede` (sonda do `curl`).
pub fn existe_no_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| existe_em(bin, &p))
}

/// **O quê:** implementação real do [`Shell`] para Unix: `<shell> -lc <cmd>`.
/// **Onde:** composição dos apps Linux/macOS/FreeBSD.
#[cfg(unix)]
pub struct ShellUnix;

#[cfg(unix)]
impl Shell for ShellUnix {
    /// **O quê:** executa `cmd` com `login_shell() -lc`, stdout/stderr capturados.
    /// **Onde:** quem usa `&dyn Shell`; efeito colateral: spawna um processo.
    fn run(&self, cmd: &str) -> io::Result<Output> {
        std::process::Command::new(login_shell()).arg("-lc").arg(cmd).output()
    }

    /// **O quê:** `true` se `bin` é executável no `PATH`.
    /// **Onde:** sondas de ferramenta instalada.
    fn existe(&self, bin: &str) -> bool {
        existe_no_path(bin)
    }

    /// **O quê:** `<login_shell> -lc <cmd>` com stdin/stdout/stderr herdados.
    /// **Onde:** `exec_step` das receitas do market.
    fn run_herdado(&self, cmd: &str) -> io::Result<ExitStatus> {
        self.executar(&[login_shell(), "-lc", cmd], true)
    }
}

#[cfg(all(test, unix))]
mod testes {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;

    /// **O quê:** cria um diretório temporário único e vazio. **Onde:** testes de PATH.
    fn tmp(nome: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("seams_shell_{nome}_{}", std::process::id()));
        if d.exists() {
            std::fs::remove_dir_all(&d).expect("limpar");
        }
        std::fs::create_dir_all(&d).expect("mkdir");
        d
    }

    /// **O quê:** grava um arquivo com o modo dado. **Onde:** testes de PATH.
    fn arq(d: &Path, nome: &str, modo: u32) {
        let p = d.join(nome);
        std::fs::write(&p, "#!/bin/sh\n").expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(modo)).expect("chmod");
    }

    /// **O quê:** prova a tabela de escolha do shell. **Onde:** `cargo test`.
    #[test]
    fn escolha_do_shell() {
        assert_eq!(escolher_shell(true, false), "sh");
        assert_eq!(escolher_shell(true, true), "bash");
        assert_eq!(escolher_shell(false, false), "bash");
        assert_eq!(escolher_shell(false, true), "bash");
    }

    /// **O quê:** executável resolve; sem bit de exec, diretório e ausente não. **Onde:** teste.
    #[test]
    fn existe_exige_bit_de_exec() {
        let d = tmp("exec");
        arq(&d, "bom", 0o755);
        arq(&d, "sem_exec", 0o644);
        std::fs::create_dir(d.join("dir")).expect("mkdir");
        let p: OsString = d.clone().into_os_string();
        assert!(existe_em("bom", &p));
        assert!(!existe_em("sem_exec", &p));
        assert!(!existe_em("dir", &p));
        assert!(!existe_em("nao_existe", &p));
        std::fs::remove_dir_all(&d).expect("rm");
    }

    /// **O quê:** nomes hostis (vazio, `../`, barra, NUL, unicode) e PATH hostil não resolvem.
    /// **Onde:** teste.
    #[test]
    fn existe_rejeita_entradas_hostis() {
        let d = tmp("hostil");
        arq(&d, "bom", 0o755);
        let p = d.clone().into_os_string();
        for ruim in ["", "../bom", "/bin/sh", "a\\b", "bo\0m", "bóm", "  "] {
            assert!(!existe_em(ruim, &p), "{ruim:?}");
        }
        assert!(!existe_em("bom", std::ffi::OsStr::new("")));
        assert!(!existe_em("bom", std::ffi::OsStr::new("::")));
        std::fs::remove_dir_all(&d).expect("rm");
    }

    /// **O quê:** vários diretórios no PATH — acha no segundo, ignora inexistente. **Onde:** teste.
    #[test]
    fn existe_percorre_varios_diretorios() {
        let a = tmp("multi_a");
        let b = tmp("multi_b");
        arq(&b, "ferr", 0o700);
        let p =
            std::env::join_paths([a.clone(), "/nao/existe/xyz".into(), b.clone()]).expect("join");
        assert!(existe_em("ferr", &p));
        std::fs::remove_dir_all(&a).expect("rm");
        std::fs::remove_dir_all(&b).expect("rm");
    }

    /// **O quê:** `sh` real existe e um binário inventado não. **Onde:** teste de `ShellUnix`.
    #[test]
    fn shell_unix_existe() {
        assert!(ShellUnix.existe("sh"));
        assert!(!ShellUnix.existe("schematize_binario_que_nao_existe_zzz"));
    }

    /// **O quê:** captura stdout/stderr e o código de saída (0 e não-zero). **Onde:** teste.
    #[test]
    fn shell_unix_run_captura() {
        let ok = ShellUnix.run("printf oi").expect("run");
        assert!(ok.status.success());
        assert_eq!(ok.stdout, b"oi");
        let ruim = ShellUnix.run("echo erro >&2; exit 3").expect("run");
        assert_eq!(ruim.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&ruim.stderr).trim(), "erro");
    }

    /// **O quê:** comando hostil (aspas, unicode, vazio, comando inexistente) não derruba o
    /// processo — vira status não-zero ou saída literal. **Onde:** teste.
    #[test]
    fn shell_unix_run_hostil() {
        let o = ShellUnix.run("printf '%s' 'ção \"x\" $((1+1))'").expect("run");
        assert_eq!(String::from_utf8_lossy(&o.stdout), "ção \"x\" $((1+1))");
        assert!(ShellUnix.run("").expect("run").status.success());
        let nf = ShellUnix.run("schematize_cmd_inexistente_zzz").expect("run");
        assert_eq!(nf.status.code(), Some(127));
    }

    /// **O quê:** `executar` roda direto (sem parsing de shell) e `run_herdado` devolve o
    /// status do comando. **Onde:** teste.
    #[test]
    fn executar_e_run_herdado() {
        assert!(ShellUnix.executar(&["true"], false).expect("true").success());
        assert_eq!(ShellUnix.executar(&["sh", "-c", "exit 3"], false).unwrap().code(), Some(3));
        // Sem parsing de shell: o `;` é argumento literal, não separador.
        assert!(ShellUnix.executar(&["echo", "a;false"], false).unwrap().success());
        assert!(ShellUnix.executar(&[], false).is_err());
        assert!(ShellUnix.executar(&["schematize_inexistente_zzz"], true).is_err());
        assert!(ShellUnix.run_herdado("exit 0").unwrap().success());
        assert_eq!(ShellUnix.run_herdado("exit 5").unwrap().code(), Some(5));
    }

    /// **O quê:** `login_shell` real é um dos dois valores válidos. **Onde:** teste.
    #[test]
    fn login_shell_valido() {
        assert!(matches!(login_shell(), "bash" | "sh"));
    }
}
