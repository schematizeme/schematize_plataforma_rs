//! Execução jaulada: spawn, monitoramento do cgroup, timeout e veredito.
//!
//! **O quê:** [`executar`] roda `argv` sob o teto de um [`Orcamento`] e devolve um [`Veredito`].
//! **Onde:** orquestradores que despacham subagents/código. Falhas viram valor, nunca pânico.

use super::backend::{detectar_backend, motivo_sem_backend, Backend};
use super::comando::{comando_jaulado, comando_prlimit, sanitizar_unidade};
use super::orcamento::Orcamento;
use super::veredito::{
    classificar, parse_anon, parse_bytes, parse_cgroup_path, parse_oom_kill, parse_result_oom,
    Evidencia, Veredito,
};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Intervalo entre amostras do cgroup.
const PASSO: Duration = Duration::from_millis(20);
/// Quanto tempo a memória anônima pode ficar acima do `MemoryHigh` antes de contar como encalhe.
const CARENCIA_ENCALHE: Duration = Duration::from_secs(3);

/// **O quê:** roda `argv` jaulado e classifica o fim. `argv` vazio ou com NUL → `Falhou(127)`
/// (não executou). Sem backend → `SemBackend` (nunca roda sem teto). Timeout mata a UNIDADE
/// (`systemctl --user kill`), nunca o processo pai.
/// **Onde:** ponto de entrada público do módulo.
pub fn executar(o: &Orcamento, argv: &[String], nome_unidade: &str) -> Veredito {
    let backend = detectar_backend();
    if backend == Backend::Nenhum {
        return Veredito::SemBackend(motivo_sem_backend());
    }
    if argv.is_empty() || argv.iter().any(|a| a.contains('\0')) {
        return Veredito::Falhou(127);
    }
    let unidade = sanitizar_unidade(nome_unidade);
    let linha = match backend {
        Backend::Prlimit => comando_prlimit(o, argv),
        _ => comando_jaulado(o, argv, &unidade),
    };
    let Ok(filho) = lancar(&linha) else { return Veredito::Falhou(127) };
    acompanhar(filho, o, &unidade, backend == Backend::SystemdScope)
}

/// **O quê:** spawn do vetor (programa + args), stdin nulo, saída herdada.
/// **Onde:** [`executar`].
fn lancar(linha: &[String]) -> std::io::Result<Child> {
    let (prog, args) = linha
        .split_first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "argv vazio"))?;
    Command::new(prog).args(args).stdin(Stdio::null()).spawn()
}

/// **O quê:** laço de espera: amostra o cgroup, aplica o timeout e classifica ao terminar.
/// **Onde:** [`executar`].
fn acompanhar(mut filho: Child, o: &Orcamento, unidade: &str, cgroup: bool) -> Veredito {
    let inicio = Instant::now();
    let mut ev = Evidencia::default();
    let mut dir: Option<String> = None;
    let mut acima_desde: Option<Instant> = None;
    loop {
        if cgroup {
            amostrar(filho.id(), unidade, &mut dir, &mut ev);
            if encalhado(o, &ev, &mut acima_desde) {
                ev.encalhou = true;
                matar_unidade(&mut filho, unidade, cgroup);
                return classificar(None, Some(9), &ev, o.mem_max_mib);
            }
        }
        match filho.try_wait() {
            Ok(Some(st)) => return fim(st.code(), st.signal(), ev, o, unidade, cgroup),
            Ok(None) => {}
            Err(_) => return Veredito::Falhou(1),
        }
        if o.timeout.is_some_and(|t| inicio.elapsed() >= t) {
            matar_unidade(&mut filho, unidade, cgroup);
            return Veredito::Timeout;
        }
        std::thread::sleep(PASSO);
    }
}

/// **O quê:** `true` se a memória anônima ficou >= `MemoryHigh` por mais de [`CARENCIA_ENCALHE`].
/// Sem swap o kernel não recupera anônima: só faz throttle e o corte no `MemoryMax` demora
/// uma eternidade; matar a unidade aqui mantém o contrato "estourou → morre só ela".
/// **Onde:** [`acompanhar`].
fn encalhado(o: &Orcamento, ev: &Evidencia, desde: &mut Option<Instant>) -> bool {
    let acima = ev.anon_bytes.is_some_and(|b| b >= o.mem_high_mib * 1024 * 1024);
    if !acima {
        *desde = None;
        return false;
    }
    desde.get_or_insert_with(Instant::now).elapsed() >= CARENCIA_ENCALHE
}

/// **O quê:** fecha o veredito; se foi SIGKILL sem prova, consulta `systemctl show -p Result`.
/// **Onde:** [`acompanhar`].
fn fim(
    code: Option<i32>,
    sig: Option<i32>,
    mut ev: Evidencia,
    o: &Orcamento,
    unidade: &str,
    cgroup: bool,
) -> Veredito {
    if cgroup && ev.oom_kill == 0 && (sig == Some(9) || code == Some(137)) {
        ev.result_oom = consultar_result(unidade);
    }
    classificar(code, sig, &ev, o.mem_max_mib)
}

/// **O quê:** uma amostra: localiza o diretório do cgroup (uma vez) e lê events/peak/current.
/// **Onde:** [`acompanhar`], a cada passo.
fn amostrar(pid: u32, unidade: &str, dir: &mut Option<String>, ev: &mut Evidencia) {
    if dir.is_none() {
        let texto = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
        *dir = parse_cgroup_path(&texto).filter(|p| p.contains(unidade));
    }
    let Some(d) = dir.as_deref() else { return };
    let base = format!("/sys/fs/cgroup{d}");
    if let Some(n) = std::fs::read_to_string(format!("{base}/memory.events"))
        .ok()
        .and_then(|t| parse_oom_kill(&t))
    {
        ev.oom_kill = ev.oom_kill.max(n);
    }
    let stat = std::fs::read_to_string(format!("{base}/memory.stat")).ok();
    ev.anon_bytes = stat.as_deref().and_then(parse_anon);
    for arq in ["memory.peak", "memory.current"] {
        let lido = std::fs::read_to_string(format!("{base}/{arq}")).ok();
        if let Some(b) = lido.as_deref().and_then(parse_bytes) {
            ev.pico_bytes = Some(ev.pico_bytes.map_or(b, |p| p.max(b)));
        }
    }
}

/// **O quê:** `true` se `systemctl --user show <unit>.scope -p Result` diz `oom-kill`.
/// **Onde:** [`fim`].
fn consultar_result(unidade: &str) -> bool {
    let saida = Command::new("systemctl")
        .args(["--user", "show", &format!("{unidade}.scope"), "-p", "Result"])
        .stdin(Stdio::null())
        .output();
    saida.is_ok_and(|s| parse_result_oom(&String::from_utf8_lossy(&s.stdout)))
}

/// **O quê:** mata a unidade (SIGKILL via systemd); se o systemd não deu conta, mata o `filho`
/// direto, e reapa. O pai (orquestrador) nunca é alvo: `filho` é o processo jaulado.
/// **Onde:** [`acompanhar`] no timeout.
fn matar_unidade(filho: &mut Child, unidade: &str, cgroup: bool) {
    let via_systemd = cgroup
        && Command::new("systemctl")
            .args(["--user", "kill", "--signal=SIGKILL", &format!("{unidade}.scope")])
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    if !via_systemd {
        if let Err(e) = filho.kill() {
            eprintln!("jaula: kill do filho falhou (provavelmente já morreu): {e}");
        }
    }
    if let Err(e) = filho.wait() {
        eprintln!("jaula: wait do filho falhou: {e}");
    }
}
