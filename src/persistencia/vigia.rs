//! `Vigia` (watchdog): processo SEPARADO que lança o orquestrador e o relança com `--resume`
//! quando ele morre (SIGKILL/OOM/exit anormal), com backoff exponencial + jitter e circuit
//! breaker. O integrador cria o `[[bin]] schematize-vigia` chamando [`rodar`].

use super::aleatorio::Rng;
use super::erro::Erro;
use super::fsutil::escreve_atomico_bin;
use super::journal::{Config, Durabilidade, Journal};
use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

const T_LANCAMENTO: u8 = 1;
const T_MORTE: u8 = 2;
const T_FIM: u8 = 3;
const T_PARKED: u8 = 4;

/// **O quê:** configuração do vigia. **Onde:** `rodar` e `unit_systemd`.
#[derive(Clone, Debug)]
pub struct VigiaCfg {
    /// Binário do próprio vigia (vai no `ExecStart` da unit).
    pub exe_vigia: PathBuf,
    /// Comando do orquestrador a vigiar.
    pub comando: PathBuf,
    pub args: Vec<String>,
    /// Argumento acrescentado em todo relançamento (default `--resume`).
    pub arg_resume: String,
    /// Diretório de estado do vigia (journal próprio em `vigia/`, marcador `parked`).
    pub dir_estado: PathBuf,
    /// Mortes dentro de `janela` que abrem o circuito (>=).
    pub max_mortes: u32,
    pub janela: Duration,
    pub backoff_base: Duration,
    pub backoff_teto: Duration,
    /// `oom_score_adj` desejado para o orquestrador (-500..-900; ignorado se não permitido).
    pub oom_score_adj: i32,
    /// `memory.events` a observar; `None` = descobre pelo cgroup do filho.
    pub memory_events: Option<PathBuf>,
    pub semente: u64,
    /// `MemoryMax` do slice na unit systemd (folgado: o orquestrador roda dentro dele).
    pub memoria_max_slice: String,
}

impl VigiaCfg {
    /// **O quê:** defaults prudentes (5 mortes/10 min, backoff 1 s..60 s, adj -800).
    ///
    /// **Onde:** construção típica pelo `main` do vigia.
    pub fn novo(comando: impl Into<PathBuf>, dir_estado: impl Into<PathBuf>) -> Self {
        VigiaCfg {
            exe_vigia: PathBuf::from("schematize-vigia"),
            comando: comando.into(),
            args: Vec::new(),
            arg_resume: "--resume".into(),
            dir_estado: dir_estado.into(),
            max_mortes: 5,
            janela: Duration::from_secs(600),
            backoff_base: Duration::from_secs(1),
            backoff_teto: Duration::from_secs(60),
            oom_score_adj: -800,
            memory_events: None,
            semente: 0,
            memoria_max_slice: "4G".into(),
        }
    }
}

/// **O quê:** como o vigia terminou. **Onde:** retorno de `rodar` (vira exit code do bin).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Desfecho {
    /// O orquestrador saiu com sucesso.
    Concluido { lancamentos: u32 },
    /// Circuito aberto: marcador `parked` gravado, vigia parou.
    Parked { mortes: u32 },
    /// Já havia `parked` de antes; nada foi lançado (apague o marcador para retomar).
    JaParked,
}

/// **O quê:** registro de uma morte do orquestrador. **Onde:** journal do vigia.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Morte {
    pub sinal: Option<i32>,
    pub codigo: Option<i32>,
    pub causa: String,
    pub vivo_ms: u128,
}

impl Morte {
    /// **O quê:** linha `chave=valor` gravada no journal. **Onde:** `rodar`.
    pub fn linha(&self) -> String {
        let f = |o: Option<i32>| o.map_or("-".to_string(), |v| v.to_string());
        format!(
            "morte sinal={} codigo={} causa={} vivo_ms={}",
            f(self.sinal),
            f(self.codigo),
            self.causa,
            self.vivo_ms
        )
    }
}

/// **O quê:** classifica o término: sinal/código e causa provável (OOM se o contador
/// `oom_kill` do cgroup subiu). **Onde:** `rodar` após cada `wait`.
pub fn classificar(
    st: ExitStatus,
    oom_antes: Option<u64>,
    oom_depois: Option<u64>,
    vivo: Duration,
) -> Morte {
    let (sinal, codigo) = (st.signal(), st.code());
    let oom = matches!((oom_antes, oom_depois), (Some(a), Some(d)) if d > a);
    let causa = match (sinal, codigo) {
        (Some(_), _) if oom => "oom_kill_cgroup".to_string(),
        (Some(9), _) => "sigkill_sem_oom_visivel".to_string(),
        (Some(s), _) => format!("sinal_{s}"),
        (None, Some(c)) => format!("exit_{c}"),
        (None, None) => "desconhecida".to_string(),
    };
    Morte { sinal, codigo, causa, vivo_ms: vivo.as_millis() }
}

/// **O quê:** extrai o contador `oom_kill` do texto de `memory.events`. **Onde:** `ler_oom_kill`.
pub fn parse_oom_kill(conteudo: &str) -> Option<u64> {
    conteudo.lines().find_map(|l| l.strip_prefix("oom_kill ")?.trim().parse().ok())
}

/// **O quê:** caminho do `memory.events` a partir do texto de `/proc/<pid>/cgroup` (cgroup v2).
///
/// **Onde:** `ler_oom_kill`.
pub fn caminho_memory_events(proc_cgroup: &str) -> Option<PathBuf> {
    let rel = proc_cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    Some(PathBuf::from(format!("/sys/fs/cgroup{}/memory.events", rel.trim())))
}

/// **O quê:** `oom_kill` atual do cgroup do filho (ou de `forcado`); `None` se ilegível.
///
/// **Onde:** baseline logo após o spawn e leitura após o `wait`. Ausência de cgroup v2 não
/// é erro: só significa "sem evidência de OOM" (a causa fica `sigkill_sem_oom_visivel`).
fn ler_oom_kill(pid: u32, forcado: &Option<PathBuf>, cache: &mut Option<PathBuf>) -> Option<u64> {
    if cache.is_none() {
        *cache = forcado.clone().or_else(|| {
            let c = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
            caminho_memory_events(&c)
        });
    }
    parse_oom_kill(&std::fs::read_to_string(cache.as_ref()?).ok()?)
}

/// **O quê:** tenta gravar `oom_score_adj` do processo `pid`; devolve se foi aplicado.
///
/// **Onde:** logo após lançar o orquestrador (e no próprio vigia). Sem permissão para valor
/// negativo é esperado (precisa de CAP_SYS_RESOURCE): vira `false` no registro, sem erro.
pub fn ajustar_oom(pid: u32, valor: i32) -> bool {
    std::fs::write(format!("/proc/{pid}/oom_score_adj"), valor.to_string()).is_ok()
}

/// **O quê:** atraso antes do relançamento: `base * 2^(mortes-1)` limitado ao teto, mais
/// jitter uniforme em `[0, atraso/2]`. **Onde:** `rodar`.
pub fn backoff(cfg: &VigiaCfg, mortes_na_janela: u32, rng: &mut Rng) -> Duration {
    let exp = mortes_na_janela.saturating_sub(1).min(20);
    let bruto = cfg.backoff_base.saturating_mul(1u32 << exp).min(cfg.backoff_teto);
    let jitter_ms = rng.ate(bruto.as_millis() as u64 / 2 + 1);
    bruto + Duration::from_millis(jitter_ms)
}

/// **O quê:** abre o journal próprio do vigia (segmentos pequenos para o RSS ficar baixo).
///
/// **Onde:** início de `rodar`. Lock exclusivo: dois vigias no mesmo estado dão `LockOcupado`.
fn abre_diario(dir: &Path) -> Result<Journal, Erro> {
    let cfg = Config { durabilidade: Durabilidade::Cada, tamanho_segmento: 64 << 10 };
    let mut j = Journal::abrir(&dir.join("vigia"), cfg)?;
    j.recuperar_desde(u64::MAX)?;
    let corte = j.proximo_seq().saturating_sub(1000);
    j.compactar(corte)?;
    Ok(j)
}

/// **O quê:** lança o orquestrador (com `--resume` nos relançamentos).
///
/// **Onde:** `ciclo`.
fn lancar(cfg: &VigiaCfg, resume: bool) -> std::io::Result<Child> {
    let mut c = Command::new(&cfg.comando);
    c.args(&cfg.args);
    if resume {
        c.arg(&cfg.arg_resume);
    }
    c.spawn()
}

/// **O quê:** um ciclo lançar -> esperar -> classificar; registra o lançamento no journal.
///
/// **Onde:** laço de `rodar`. Falha de spawn vira morte `falha_spawn` (para o circuito acabar
/// abrindo em vez de girar para sempre).
fn ciclo(
    cfg: &VigiaCfg,
    resume: bool,
    n: u32,
    diario: &mut Journal,
) -> Result<(Morte, bool), Erro> {
    let inicio = Instant::now();
    let mut filho = match lancar(cfg, resume) {
        Ok(f) => f,
        Err(e) => {
            let causa = format!("falha_spawn:{e}").replace(' ', "_");
            return Ok((Morte { sinal: None, codigo: None, causa, vivo_ms: 0 }, false));
        }
    };
    let aplicado = ajustar_oom(filho.id(), cfg.oom_score_adj);
    let (mut cache, pid) = (None, filho.id());
    let antes = ler_oom_kill(pid, &cfg.memory_events, &mut cache);
    let msg = format!("lancamento n={n} resume={resume} pid={pid} oom_adj_aplicado={aplicado}");
    diario.anexar(T_LANCAMENTO, msg.as_bytes())?;
    let st = filho.wait().map_err(super::erro::io("esperar o orquestrador"))?;
    let depois = ler_oom_kill(pid, &cfg.memory_events, &mut cache);
    Ok((classificar(st, antes, depois, inicio.elapsed()), st.success()))
}

/// **O quê:** grava o marcador `parked` com o motivo. **Onde:** circuito aberto em `rodar`.
fn grava_parked(cfg: &VigiaCfg, mortes: u32, ultima: &Morte) -> Result<(), Erro> {
    let txt = format!(
        "parked mortes={mortes} janela_s={} ultima={}\n",
        cfg.janela.as_secs(),
        ultima.linha()
    );
    escreve_atomico_bin(&cfg.dir_estado.join("parked"), txt.as_bytes())
}

/// **O quê:** laço principal do vigia: lança, espera, relança com backoff até concluir com
/// sucesso ou o circuito abrir (`max_mortes` na `janela`), quando grava `parked` e PARA.
///
/// **Onde:** `main` do `schematize-vigia`. Cada lançamento e morte (sinal, exit, causa
/// provável, tempo vivo) vai para o journal do próprio vigia.
pub fn rodar(cfg: &VigiaCfg) -> Result<Desfecho, Erro> {
    let mut diario = abre_diario(&cfg.dir_estado)?;
    if cfg.dir_estado.join("parked").exists() {
        return Ok(Desfecho::JaParked);
    }
    ajustar_oom(std::process::id(), cfg.oom_score_adj);
    let mut rng = Rng::novo(cfg.semente ^ u64::from(std::process::id()));
    let mut recentes: VecDeque<Instant> = VecDeque::new();
    let mut n = 0u32;
    loop {
        let (morte, sucesso) = ciclo(cfg, n > 0, n, &mut diario)?;
        n += 1;
        if sucesso {
            diario.anexar(T_FIM, format!("fim_ok lancamentos={n}").as_bytes())?;
            return Ok(Desfecho::Concluido { lancamentos: n });
        }
        diario.anexar(T_MORTE, morte.linha().as_bytes())?;
        let agora = Instant::now();
        recentes.push_back(agora);
        while recentes.front().is_some_and(|t| agora.duration_since(*t) > cfg.janela) {
            recentes.pop_front();
        }
        let k = recentes.len() as u32;
        if k >= cfg.max_mortes {
            diario.anexar(T_PARKED, format!("parked {}", morte.linha()).as_bytes())?;
            grava_parked(cfg, k, &morte)?;
            return Ok(Desfecho::Parked { mortes: k });
        }
        std::thread::sleep(backoff(cfg, k, &mut rng));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Executa `sh -c script` e devolve o status. **Onde:** testes de `classificar`.
    fn status(script: &str) -> ExitStatus {
        Command::new("/bin/sh").args(["-c", script]).status().expect("sh existe")
    }

    /// Classificação: exit, SIGKILL com e sem evidência de OOM.
    #[test]
    fn classifica_causas() {
        let v = Duration::from_millis(5);
        assert_eq!(classificar(status("exit 3"), None, None, v).causa, "exit_3");
        let k = classificar(status("kill -9 $$"), Some(1), Some(1), v);
        assert_eq!((k.sinal, k.causa.as_str()), (Some(9), "sigkill_sem_oom_visivel"));
        assert_eq!(classificar(status("kill -9 $$"), Some(1), Some(2), v).causa, "oom_kill_cgroup");
        assert_eq!(classificar(status("kill -15 $$"), None, None, v).causa, "sinal_15");
    }

    /// Parsers puros de memory.events e /proc/pid/cgroup.
    #[test]
    fn parsers() {
        assert_eq!(parse_oom_kill("low 0\nhigh 0\noom 2\noom_kill 3\n"), Some(3));
        assert_eq!(parse_oom_kill("lixo"), None);
        assert_eq!(
            caminho_memory_events("0::/user.slice/x.scope\n"),
            Some(PathBuf::from("/sys/fs/cgroup/user.slice/x.scope/memory.events"))
        );
        assert_eq!(caminho_memory_events("1:cpu:/a\n"), None);
    }

    /// Backoff cresce, respeita o teto (+ até 50% de jitter) e é determinístico por seed.
    #[test]
    fn backoff_exponencial_com_teto() {
        let mut cfg = VigiaCfg::novo("x", "y");
        cfg.backoff_base = Duration::from_millis(100);
        cfg.backoff_teto = Duration::from_millis(1000);
        let mut r = Rng::novo(1);
        let b1 = backoff(&cfg, 1, &mut r);
        assert!(b1 >= Duration::from_millis(100) && b1 <= Duration::from_millis(150));
        let b9 = backoff(&cfg, 9, &mut r);
        assert!(b9 >= Duration::from_millis(1000) && b9 <= Duration::from_millis(1500));
        assert_eq!(backoff(&cfg, 3, &mut Rng::novo(5)), backoff(&cfg, 3, &mut Rng::novo(5)));
    }
}
