//! Testes de `processos`: parsers de todos os SOs com fixtures e motor com `/proc`/comandos falsos.
//!
//! **O quê:** nenhum assert de conteúdo lê o `/proc` nem roda o `ps` reais; só o smoke final
//! cria um `sleep` próprio e o consulta.
//!
//! **Onde:** `cargo test --features seams`.

use super::parse::*;
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// **O quê:** `/proc` falso: pids, cmdlines, cwds e textos fixos.
///
/// **Onde:** os testes do ramo Linux.
#[derive(Default)]
struct ProcFalso {
    listavel: bool,
    cmdlines: HashMap<u32, Vec<u8>>,
    cwds: HashMap<u32, PathBuf>,
    textos: HashMap<String, String>,
}

impl ProcFs for ProcFalso {
    /// **O quê:** pids das chaves de cmdline (ou `None` se `listavel` é falso).
    ///
    /// **Onde:** trait [`ProcFs`] falso.
    fn pids(&self) -> Option<Vec<u32>> {
        self.listavel.then(|| self.cmdlines.keys().copied().collect())
    }
    /// **O quê:** existe se tem cmdline.
    ///
    /// **Onde:** trait [`ProcFs`] falso.
    fn existe(&self, pid: u32) -> bool {
        self.cmdlines.contains_key(&pid)
    }
    /// **O quê:** cmdline da fixture.
    ///
    /// **Onde:** trait [`ProcFs`] falso.
    fn cmdline(&self, pid: u32) -> Option<Vec<u8>> {
        self.cmdlines.get(&pid).cloned()
    }
    /// **O quê:** cwd da fixture.
    ///
    /// **Onde:** trait [`ProcFs`] falso.
    fn cwd(&self, pid: u32) -> Option<PathBuf> {
        self.cwds.get(&pid).cloned()
    }
    /// **O quê:** texto da fixture.
    ///
    /// **Onde:** trait [`ProcFs`] falso.
    fn texto(&self, arquivo: &str) -> Option<String> {
        self.textos.get(arquivo).cloned()
    }
}

/// **O quê:** executor falso: responde pelo prefixo de `"prog args"` e registra as chamadas.
///
/// **Onde:** os testes dos ramos FreeBSD/macOS/Windows.
#[derive(Default)]
struct CmdFalso {
    respostas: Vec<(String, String)>,
    chamadas: Mutex<Vec<String>>,
}

impl Executor for CmdFalso {
    /// **O quê:** acha a 1a resposta cujo prefixo casa; sem resposta = programa inexistente.
    ///
    /// **Onde:** trait [`Executor`] falso.
    fn rodar(&self, prog: &str, args: &[&str]) -> Option<String> {
        let chave = format!("{prog} {}", args.join(" "));
        self.chamadas.lock().ok()?.push(chave.clone());
        self.respostas.iter().find(|(p, _)| chave.starts_with(p.as_str())).map(|(_, o)| o.clone())
    }
}

/// **O quê:** monta o motor para `sistema` com o `/proc` e as respostas dadas.
///
/// **Onde:** quase todos os testes de motor.
fn motor(
    sistema: Sistema,
    proc: ProcFalso,
    respostas: &[(&str, &str)],
) -> (ProcessosCom, Arc<CmdFalso>) {
    let cmd = Arc::new(CmdFalso {
        respostas: respostas.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        chamadas: Mutex::new(Vec::new()),
    });
    (ProcessosCom::novo(sistema, Arc::new(proc), cmd.clone()), cmd)
}

/// **O quê:** `/proc` falso com processos `(pid, cmdline com NUL, cwd)`.
///
/// **Onde:** os testes do ramo Linux.
fn proc_com(itens: &[(u32, &[u8], &str)]) -> ProcFalso {
    let mut p = ProcFalso { listavel: true, ..Default::default() };
    for (pid, cmd, cwd) in itens {
        p.cmdlines.insert(*pid, cmd.to_vec());
        p.cwds.insert(*pid, PathBuf::from(cwd));
    }
    p
}

/// Saída de `ps -axo pid=,comm=,args=` (mistura de SOs, com lixo e cabeçalho).
const PS: &str = "  PID COMM ARGS\n    1 init /sbin/init\n  200 claude claude --resume\n  201 node node /usr/lib/claude/cli.js\n  202 node node --mcp /home/u/.claude/x.json\n  203 vim vim /home/u/.claude/notas\n  204 bash\n\u{0}garbage\n 99999999999 big big\n";

/// **O quê:** `cmdline_casa` reproduz as regras do `cmdline_e_claude` original.
///
/// **Onde:** regressão da migração.
#[test]
fn cmdline_casa_regras_originais() {
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(cmdline_casa(&a(&["/usr/bin/claude"]), "claude"));
    assert!(cmdline_casa(&a(&["C:\\bin\\claude"]), "claude"));
    assert!(cmdline_casa(&a(&["node", "/x/claude/cli.js"]), "claude"));
    assert!(cmdline_casa(&a(&["bun", "x", "claude-code"]), "claude"));
    assert!(!cmdline_casa(&a(&["node", "--mcp", "/home/u/.claude/x.json"]), "claude"));
    assert!(!cmdline_casa(&a(&["vim", "/home/u/.claude/n"]), "claude"));
    assert!(!cmdline_casa(&a(&["schematize", "claude"]), "claude"));
    assert!(!cmdline_casa(&[], "claude"));
    assert!(!cmdline_casa(&a(&["claude"]), ""));
}

/// **O quê:** cmdline hostil (NUL só, não-UTF-8, NULs repetidos) não derruba o parser.
///
/// **Onde:** parsers de cmdline.
#[test]
fn args_de_cmdline_hostil() {
    assert!(args_de_cmdline(b"").is_empty());
    assert!(args_de_cmdline(&[0, 0, 0]).is_empty());
    assert_eq!(args_de_cmdline(b"a\0\0b\0"), vec!["a", "b"]);
    let v = args_de_cmdline(&[0xff, 0xfe, 0, b'x']);
    assert_eq!(v.len(), 2);
    assert!(v[0].contains('\u{fffd}'));
}

/// **O quê:** `parse_ps` pula cabeçalho/lixo/pid estourado e usa `comm` quando não há args.
///
/// **Onde:** parser do `ps`.
#[test]
fn parse_ps_pula_lixo() {
    let l = parse_ps(PS);
    let pids: Vec<u32> = l.iter().map(|(p, _)| *p).collect();
    assert_eq!(pids, vec![1, 200, 201, 202, 203, 204]);
    assert_eq!(l[5].1, vec!["bash"]);
    assert_eq!(l[1].1, vec!["claude", "--resume"]);
    assert!(parse_ps("").is_empty());
    assert!(parse_ps("-1 x y\n12abc x\n").is_empty());
}

/// **O quê:** `pid_em_ps` exige pid exato (não prefixo) e rejeita lixo.
///
/// **Onde:** `vivo` fora do Linux.
#[test]
fn pid_em_ps_exato() {
    assert!(pid_em_ps("  123\n", 123));
    assert!(!pid_em_ps("  1234\n", 123));
    assert!(!pid_em_ps("", 123));
    assert!(!pid_em_ps("abc\n-5\n", 5));
}

/// **O quê:** `lsof -Fn` e `procstat -f` viram cwds; sem `cwd`/sem `/` é ignorado.
///
/// **Onde:** parsers de cwd.
#[test]
fn parsers_de_cwd() {
    let lsof = "p200\nfcwd\nn/home/u/proj\np201\nfcwd\nn/home/u/com espaço\nn\n";
    assert_eq!(
        parse_lsof_cwd(lsof),
        vec![PathBuf::from("/home/u/proj"), PathBuf::from("/home/u/com espaço")]
    );
    let ps = "  PID COMM FD T V FLAGS REF OFFSET PRO NAME\n 200 claude cwd v d r------- - - - /home/u/proj\n 200 claude root v d r------- - - - /\n 201 node cwd v d r------- - - - /a b/c\n 202 x cwd v d - - - - -\n";
    assert_eq!(
        parse_procstat_cwd(ps),
        vec![PathBuf::from("/home/u/proj"), PathBuf::from("/a b/c")]
    );
    assert!(parse_procstat_cwd("").is_empty() && parse_lsof_cwd("").is_empty());
}

/// **O quê:** memória Linux: caso normal, ausente, negativo, gigante e lixo.
///
/// **Onde:** `parse_meminfo_mb`.
#[test]
fn meminfo_casos() {
    assert_eq!(parse_meminfo_mb("MemTotal: 1 kB\nMemAvailable:   12345678 kB\n"), Some(12056));
    assert_eq!(parse_meminfo_mb("MemTotal: 1 kB\n"), None);
    assert_eq!(parse_meminfo_mb("MemAvailable: -5 kB\n"), None);
    assert_eq!(parse_meminfo_mb("MemAvailable: abc\n"), None);
    assert_eq!(parse_meminfo_mb("MemAvailable: 99999999999999999999999 kB\n"), None);
    assert_eq!(parse_meminfo_mb("MemAvailable: 18446744073709551615 kB\n"), Some(u64::MAX / 1024));
    assert_eq!(parse_meminfo_mb(""), None);
}

/// **O quê:** memória FreeBSD: soma free+inactive; entradas ruins dão `None`; sem overflow.
///
/// **Onde:** `parse_freebsd_mb`.
#[test]
fn freebsd_memoria() {
    assert_eq!(parse_freebsd_mb("4096\n200000\n56000\n"), Some(1000));
    assert_eq!(parse_freebsd_mb("4096\n200000\n"), None);
    assert_eq!(parse_freebsd_mb("0\n1\n1\n"), None);
    assert_eq!(parse_freebsd_mb("4096\nx\n1\n"), None);
    assert_eq!(parse_freebsd_mb("-4096\n1\n1\n"), None);
    let enorme = format!("{}\n{}\n{}\n", u128::MAX, u128::MAX, u128::MAX);
    assert_eq!(parse_freebsd_mb(&enorme), Some(u64::MAX));
}

/// **O quê:** memória macOS: free+inactive+speculative; speculative opcional; faltas dão `None`.
///
/// **Onde:** `parse_vm_stat_mb`.
#[test]
fn macos_memoria() {
    let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               10000.\nPages active:                             5.\nPages inactive:                           6384.\nPages speculative:                        0.\n";
    assert_eq!(parse_vm_stat_mb(vm), Some(256));
    let sem_spec = vm.replace("Pages speculative:                        0.\n", "");
    assert_eq!(parse_vm_stat_mb(&sem_spec), Some(256));
    assert_eq!(parse_vm_stat_mb(&vm.replace("Pages free:", "Pages freed:")), None);
    assert_eq!(parse_vm_stat_mb("Pages free: 1.\nPages inactive: 1.\n"), None);
    assert_eq!(parse_vm_stat_mb(&vm.replace("16384", "0")), None);
    assert_eq!(parse_vm_stat_mb(""), None);
}

/// **O quê:** load: formatos Linux e sysctl; NaN/inf/negativo/lixo viram `None`.
///
/// **Onde:** `parse_carga`.
#[test]
fn carga_casos() {
    assert_eq!(parse_carga("0.52 0.58 0.59 1/500 1234\n"), Some(0.52));
    assert_eq!(parse_carga("{ 1.50 0.58 0.59 }\n"), Some(1.5));
    for ruim in ["NaN 1 1", "inf", "-1.0 2", "abc", "", "{ }", "1e999"] {
        assert_eq!(parse_carga(ruim), None, "{ruim}");
    }
}

/// **O quê:** tasklist CSV: linhas válidas, INFO e lixo.
///
/// **Onde:** `parse_tasklist`.
#[test]
fn tasklist_csv() {
    let t = "\"claude.exe\",\"4242\",\"Console\",\"1\",\"10 K\"\r\nINFO: No tasks are running\n\"x.exe\",\"abc\"\n\"\"\n";
    assert_eq!(parse_tasklist(t), vec![("claude.exe".to_string(), 4242)]);
}

/// **O quê:** Linux: vivo/contar/em_dir sobre `/proc` falso, incluindo hostis.
///
/// **Onde:** ramo Linux do motor.
#[test]
fn linux_motor() {
    let proc = proc_com(&[
        (10, b"claude\0--x\0", "/p/a"),
        (11, b"node\0/usr/lib/claude/cli.js\0", "/p/b"),
        (12, b"vim\0/h/.claude/n\0", "/p/a"),
        (13, b"", "/p/a"),
        (14, &[0xff, 0xfe, 0], "/p/a"),
        (15, b"/opt/claude\0", "/p/a/sub"),
    ]);
    let (m, cmd) = motor(Sistema::Linux, proc, &[]);
    assert_eq!(m.contar("claude"), 3);
    assert_eq!(m.contar(""), 0);
    assert!(m.em_dir("claude", Path::new("/p/a"), Modo::Exato));
    assert!(m.em_dir("claude", Path::new("/p/a/"), Modo::Exato), "barra final é o mesmo caminho");
    assert!(m.em_dir("claude", Path::new("/p/a/sub"), Modo::Exato));
    assert!(m.em_dir("claude", Path::new("/p/b"), Modo::Exato));
    assert!(!m.em_dir("claude", Path::new("/p"), Modo::Exato), "ancestral não é cwd igual");
    assert!(!m.em_dir("vim-nao", Path::new("/p/a"), Modo::Exato));
    assert!(m.vivo(10) && m.vivo(13) && !m.vivo(99));
    assert!(!m.vivo(0) && !m.vivo(u32::MAX) && !m.vivo(i32::MAX as u32 + 1));
    assert!(cmd.chamadas.lock().map(|c| c.is_empty()).unwrap_or(false), "Linux não roda comando");
}

/// **O quê:** Linux sem `/proc` listável ou sem permissão: zero/false, sem pânico.
///
/// **Onde:** degradação graciosa.
#[test]
fn linux_sem_proc() {
    let (m, _) = motor(Sistema::Linux, ProcFalso::default(), &[]);
    assert_eq!(m.contar("claude"), 0);
    assert!(!m.em_dir("claude", Path::new("/"), Modo::Exato));
    assert_eq!(m.memoria_disponivel_mb(), None);
    assert_eq!(m.carga1(), None);
}

/// **O quê:** Linux: meminfo e loadavg vêm do texto injetado.
///
/// **Onde:** recursos no Linux.
#[test]
fn linux_recursos() {
    let mut p = ProcFalso::default();
    p.textos.insert("meminfo".into(), "MemAvailable: 2048 kB\n".into());
    p.textos.insert("loadavg".into(), "3.25 1 1 1/1 1\n".into());
    let (m, _) = motor(Sistema::Linux, p, &[]);
    assert_eq!(m.memoria_disponivel_mb(), Some(2));
    assert_eq!(m.carga1(), Some(3.25));
}

/// **O quê:** FreeBSD: ps -p, contar, em_dir via procstat, memória e load via sysctl.
///
/// **Onde:** ramo FreeBSD (rodado no Linux).
#[test]
fn freebsd_motor() {
    let procstat = " 200 claude cwd v d r------- - - - /home/u/proj\n 201 node cwd v d r------- - - - /outro\n";
    let (m, cmd) = motor(
        Sistema::FreeBsd,
        ProcFalso::default(),
        &[
            ("ps -p 200 ", "  200\n"),
            ("ps -p 7 ", ""),
            ("ps -axo", PS),
            ("procstat -f 200 201", procstat),
            ("sysctl -n hw.pagesize", "4096\n262144\n0\n"),
            ("sysctl -n vm.loadavg", "{ 0.75 0.5 0.4 }\n"),
        ],
    );
    assert!(m.vivo(200) && !m.vivo(7) && !m.vivo(0) && !m.vivo(u32::MAX));
    assert_eq!(m.contar("claude"), 2);
    assert!(m.em_dir("claude", Path::new("/home/u/proj"), Modo::Exato));
    assert!(m.em_dir("claude", Path::new("/outro"), Modo::Exato));
    assert!(!m.em_dir("claude", Path::new("/home/u"), Modo::Exato));
    assert_eq!(m.memoria_disponivel_mb(), Some(1024));
    assert_eq!(m.carga1(), Some(0.75));
    let c = cmd.chamadas.lock().map(|c| c.clone()).unwrap_or_default();
    assert!(c.iter().any(|x| x.starts_with("procstat -f 200 201")), "{c:?}");
}

/// **O quê:** macOS: em_dir via lsof com pids em vírgula, memória via vm_stat, load via sysctl.
///
/// **Onde:** ramo macOS (rodado no Linux).
#[test]
fn macos_motor() {
    let vm = "Mach Virtual Memory Statistics: (page size of 4096 bytes)\nPages free: 131072.\nPages inactive: 131072.\n";
    let (m, cmd) = motor(
        Sistema::MacOs,
        ProcFalso::default(),
        &[
            ("ps -axo", PS),
            ("lsof -a -d cwd -Fn -p 200,201", "p200\nfcwd\nn/Users/u/proj\n"),
            ("vm_stat", vm),
            ("sysctl -n vm.loadavg", "{ 2.00 1 1 }"),
        ],
    );
    assert!(m.em_dir("claude", Path::new("/Users/u/proj"), Modo::Exato));
    assert!(!m.em_dir("claude", Path::new("/Users/u"), Modo::Exato));
    assert_eq!(m.memoria_disponivel_mb(), Some(1024));
    assert_eq!(m.carga1(), Some(2.0));
    assert!(!m.vivo(5), "ps sem resposta = não vivo, nunca 'assume vivo'");
    assert!(cmd.chamadas.lock().map(|c| c.len() > 3).unwrap_or(false));
}

/// **O quê:** comando ausente (None) ou saída lixo: contar 0, em_dir false, recursos `None`.
///
/// **Onde:** degradação graciosa fora do Linux.
#[test]
fn comandos_ausentes_ou_lixo() {
    for so in [Sistema::FreeBsd, Sistema::MacOs, Sistema::Outro] {
        let (m, _) = motor(so, ProcFalso::default(), &[]);
        assert_eq!(m.contar("claude"), 0);
        assert!(!m.em_dir("claude", Path::new("/"), Modo::Exato));
        assert!(!m.vivo(1));
        assert_eq!(m.memoria_disponivel_mb(), None);
        assert_eq!(m.carga1(), None);
        let (l, _) = motor(so, ProcFalso::default(), &[("", "\u{fffd}\u{0}lixo\n")]);
        assert_eq!((l.contar("claude"), l.memoria_disponivel_mb(), l.carga1()), (0, None, None));
    }
}

/// **O quê:** muitos pids são enviados em lotes de `LOTE_PIDS`, sem estourar a linha de comando.
///
/// **Onde:** `cwd_por_comando`.
#[test]
fn lotes_de_pids() {
    let ps: String = (1..=600).map(|i| format!("{i} claude claude\n")).collect();
    let (m, cmd) = motor(Sistema::MacOs, ProcFalso::default(), &[("ps -axo", &ps), ("lsof", "")]);
    assert_eq!(m.contar("claude"), 600);
    assert!(!m.em_dir("claude", Path::new("/x"), Modo::Exato));
    let lsofs = cmd
        .chamadas
        .lock()
        .map(|c| c.iter().filter(|x| x.starts_with("lsof")).count())
        .unwrap_or(0);
    assert_eq!(lsofs, 3);
}

/// **O quê:** Windows: vivo e contar via tasklist; demais métodos honestos (`false`/`None`).
///
/// **Onde:** ramo Windows (rodado no Linux).
#[test]
fn windows_motor() {
    let csv = "\"claude.exe\",\"4242\",\"Console\",\"1\",\"1 K\"\n\"Claude.EXE\",\"4243\",\"Console\",\"1\",\"1 K\"\n\"x.exe\",\"5\",\"c\",\"1\",\"1 K\"\n";
    let (m, _) = motor(
        Sistema::Windows,
        ProcFalso::default(),
        &[
            ("tasklist /FI PID eq 4242", "\"claude.exe\",\"4242\",\"Console\"\n"),
            ("tasklist /FI", "INFO: No tasks\n"),
            ("tasklist /NH", csv),
        ],
    );
    assert!(m.vivo(4242) && !m.vivo(77) && !m.vivo(0));
    assert_eq!(m.contar("claude"), 2);
    assert_eq!(m.contar(".exe"), 0);
    assert!(!m.em_dir("claude", Path::new("C:\\"), Modo::Exato));
    assert_eq!((m.memoria_disponivel_mb(), m.carga1()), (None, None));
}

/// **O quê:** `Sistema::atual` bate com o `cfg!` do SO de teste.
///
/// **Onde:** seleção em runtime.
#[test]
fn sistema_atual() {
    let esperado = if cfg!(target_os = "linux") {
        Sistema::Linux
    } else if cfg!(target_os = "macos") {
        Sistema::MacOs
    } else if cfg!(target_os = "freebsd") {
        Sistema::FreeBsd
    } else if cfg!(windows) {
        Sistema::Windows
    } else {
        Sistema::Outro
    };
    assert_eq!(Sistema::atual(), esperado);
}

/// **O quê:** smoke real: um `sleep` próprio fica vivo, é contado e (no Linux) achado pelo cwd;
/// depois de morto e colhido, deixa de estar vivo. Pid 0 e `u32::MAX` nunca estão vivos.
///
/// **Onde:** única prova contra o SO real.
#[cfg(unix)]
#[test]
fn smoke_sleep_real() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().unwrap_or_default();
    let mut filho = match Command::new("sleep").arg("30").current_dir(&dir).spawn() {
        Ok(f) => f,
        Err(e) => panic!("sem sleep: {e}"),
    };
    let real = ProcessosUnix;
    assert!(real.vivo(filho.id()));
    assert!(!real.vivo(0) && !real.vivo(u32::MAX));
    assert!(real.contar("sleep") >= 1);
    #[cfg(target_os = "linux")]
    assert!(real.em_dir("sleep", &dir, Modo::Exato));
    assert!(!real.em_dir("sleep", Path::new("/nao/existe/mesmo"), Modo::Exato));
    assert!(filho.kill().is_ok());
    assert!(filho.wait().is_ok());
    assert!(!real.vivo(filho.id()), "pid colhido não pode seguir vivo");
}

/// **O quê:** `Prefixo` conta subdiretório, mas não irmão com prefixo comum só na string;
/// `Exato` não conta subdiretório (Linux).
///
/// **Onde:** semântica de `agente_vivo_em` (Prefixo) vs `supervisor_vivo_em` (Exato).
#[test]
fn modo_prefixo_vs_exato_linux() {
    let proc = proc_com(&[(10, b"claude\0", "/a/proj/sub/deep"), (11, b"claude\0", "/a/proj2")]);
    let (m, _) = motor(Sistema::Linux, proc, &[]);
    let proj = Path::new("/a/proj");
    assert!(m.em_dir("claude", proj, Modo::Prefixo), "subdir conta");
    assert!(!m.em_dir("claude", proj, Modo::Exato), "Exato não conta subdir");
    assert!(m.em_dir("claude", Path::new("/a/proj2"), Modo::Exato));
    assert!(m.em_dir("claude", Path::new("/a/proj2/"), Modo::Prefixo));
    let so_irmao = proc_com(&[(11, b"claude\0", "/a/proj2")]);
    let (m2, _) = motor(Sistema::Linux, so_irmao, &[]);
    assert!(!m2.em_dir("claude", proj, Modo::Prefixo), "/a/proj2 não está em /a/proj");
    assert!(!m2.em_dir("claude", Path::new("/a/pro"), Modo::Prefixo));
}

/// **O quê:** mesma regra de `Modo` nos caminhos por comando (FreeBSD/procstat, macOS/lsof).
///
/// **Onde:** `cwd_por_comando`.
#[test]
fn modo_por_comando() {
    let ps = "200 claude claude\n";
    let sub = " 200 claude cwd v d r------- - - - /a/proj/sub\n";
    let irmao = " 200 claude cwd v d r------- - - - /a/proj2\n";
    let proj = Path::new("/a/proj");
    let (f, _) =
        motor(Sistema::FreeBsd, ProcFalso::default(), &[("ps -axo", ps), ("procstat", sub)]);
    assert!(f.em_dir("claude", proj, Modo::Prefixo) && !f.em_dir("claude", proj, Modo::Exato));
    let (f, _) =
        motor(Sistema::FreeBsd, ProcFalso::default(), &[("ps -axo", ps), ("procstat", irmao)]);
    assert!(!f.em_dir("claude", proj, Modo::Prefixo));
    let (m, _) = motor(
        Sistema::MacOs,
        ProcFalso::default(),
        &[("ps -axo", ps), ("lsof", "p200\nfcwd\nn/a/proj/sub\n")],
    );
    assert!(m.em_dir("claude", proj, Modo::Prefixo) && !m.em_dir("claude", proj, Modo::Exato));
}

/// **O quê:** `pids_por_argv` exclui o próprio pid mesmo presente no `/proc` falso, exige TODOS os
/// trechos (substring na linha unida) e trata argv hostil sem pânico.
///
/// **Onde:** semântica de `supervisor_vivo_em`.
#[test]
fn pids_por_argv_linux() {
    let eu = std::process::id();
    let mut proc = proc_com(&[
        (10, b"schematize\0overdev\0supervise\0", "/p"),
        (11, b"schematize\0overdev\0status\0", "/p"),
        (12, b"supervise\0", "/p"),
        (13, b"", "/p"),
        (14, b"overdev\0\0\0supervise", "/p"),
        (
            15,
            &[
                0xff, b'o', b'v', b'e', b'r', b'd', b'e', b'v', 0, b's', b'u', b'p', b'e', b'r',
                b'v', b'i', b's', b'e',
            ],
            "/p",
        ),
        (16, "ação\0überdev\0supervise\0".as_bytes(), "/p"),
        (17, b"x\0overdevsupervise\0", "/p"),
    ]);
    proc.cmdlines.insert(eu, b"schematize\0overdev\0supervise\0".to_vec());
    let (m, _) = motor(Sistema::Linux, proc, &[]);
    let mut v = m.pids_por_argv(&["overdev", "supervise"]);
    v.sort_unstable();
    assert_eq!(v, vec![10, 14, 15, 17], "sem o próprio pid; substring como o original");
    assert!(!m.pids_por_argv(&["supervise"]).contains(&eu));
    assert_eq!(m.pids_por_argv(&["überdev", "ação"]), vec![16]);
    assert!(m.pids_por_argv(&[]).is_empty());
    assert!(m.pids_por_argv(&["overdev", ""]).is_empty(), "trecho vazio é fail-closed");
    assert!(m.pids_por_argv(&["overdev", "nao-existe"]).is_empty());
    let (vazio, _) = motor(Sistema::Linux, ProcFalso::default(), &[]);
    assert!(vazio.pids_por_argv(&["overdev"]).is_empty());
}

/// **O quê:** `pids_por_argv` fora do Linux: usa `ps`, exclui o próprio pid; Windows devolve vazio.
///
/// **Onde:** ramos FreeBSD/macOS/Windows.
#[test]
fn pids_por_argv_ps_e_windows() {
    let eu = std::process::id();
    let ps = format!(
        "10 schematize schematize overdev supervise\n11 schematize schematize overdev status\n{eu} schematize schematize overdev supervise\n\u{0}lixo\n"
    );
    for so in [Sistema::FreeBsd, Sistema::MacOs, Sistema::Outro] {
        let (m, _) = motor(so, ProcFalso::default(), &[("ps -axo", &ps)]);
        assert_eq!(m.pids_por_argv(&["overdev", "supervise"]), vec![10], "{so:?}");
        assert!(m.pids_por_argv(&["nada"]).is_empty());
    }
    let (sem_ps, _) = motor(Sistema::MacOs, ProcFalso::default(), &[]);
    assert!(sem_ps.pids_por_argv(&["overdev"]).is_empty());
    let (w, _) = motor(Sistema::Windows, ProcFalso::default(), &[("tasklist", "x")]);
    assert!(w.pids_por_argv(&["overdev"]).is_empty());
}

/// **O quê:** `pids_por_argv_em`: casa argv E cwd exato; cwd errado, subdir, cwd ausente e o
/// próprio pid ficam de fora (Linux).
///
/// **Onde:** semântica completa de `supervisor_vivo_em`.
#[test]
fn pids_por_argv_em_linux() {
    let eu = std::process::id();
    let sup: &[u8] = b"schematize\0overdev\0supervise\0";
    let mut proc = proc_com(&[
        (10, sup, "/a/proj"),
        (11, sup, "/a/outro"),
        (12, sup, "/a/proj/sub"),
        (13, sup, "/a/proj2"),
        (14, b"schematize\0overdev\0status\0", "/a/proj"),
        (15, sup, "/a/proj"),
    ]);
    proc.cwds.remove(&15);
    proc.cmdlines.insert(eu, sup.to_vec());
    proc.cwds.insert(eu, PathBuf::from("/a/proj"));
    let (m, _) = motor(Sistema::Linux, proc, &[]);
    let proj = Path::new("/a/proj");
    assert_eq!(m.pids_por_argv_em(&["overdev", "supervise"], proj), vec![10]);
    assert!(m.pids_por_argv_em(&["overdev", "supervise"], Path::new("/nada")).is_empty());
    assert!(m.pids_por_argv_em(&[], proj).is_empty());
}

/// **O quê:** `pids_por_argv_em` fora do Linux (cwd por procstat/lsof, um pid por vez) e Windows.
///
/// **Onde:** ramos FreeBSD/macOS/Windows.
#[test]
fn pids_por_argv_em_ps_e_windows() {
    let ps = "10 s s overdev supervise\n11 s s overdev supervise\n";
    let proj = Path::new("/a/proj");
    let (f, _) = motor(
        Sistema::FreeBsd,
        ProcFalso::default(),
        &[
            ("ps -axo", ps),
            ("procstat -f 10", " 10 s cwd v d r------- - - - /a/proj\n"),
            ("procstat -f 11", " 11 s cwd v d r------- - - - /a/outro\n"),
        ],
    );
    assert_eq!(f.pids_por_argv_em(&["overdev", "supervise"], proj), vec![10]);
    let (m, _) = motor(
        Sistema::MacOs,
        ProcFalso::default(),
        &[("ps -axo", ps), ("lsof -a -d cwd -Fn -p 11", "p11\nfcwd\nn/a/proj\n")],
    );
    assert_eq!(m.pids_por_argv_em(&["overdev", "supervise"], proj), vec![11]);
    let (w, _) = motor(Sistema::Windows, ProcFalso::default(), &[]);
    assert!(w.pids_por_argv_em(&["overdev"], proj).is_empty());
}
