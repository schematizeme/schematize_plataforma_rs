//! [`Observador`]: máquina de estados do monitor, SEM thread e SEM relógio próprio.
//!
//! **O quê:** a cada [`Observador::amostrar`] lê host e alvos por uma [`Fonte`] e devolve os
//! [`Evento`]s da borda. Contadores que regridem (reinício do cgroup) só reiniciam a linha de
//! base; nada de delta negativo. Tempo entra por parâmetro (`agora_ms`), então é determinístico.
//!
//! **Onde:** [`super::monitor::Monitor`] chama em loop; testes chamam direto com fixtures.

use super::anel::Anel;
use super::evento::{Alvo, ConfigMonitor, Evento, Origem, Recurso};
use super::fonte::Fonte;
use super::leitura::{Leitura, CGROUP_RAIZ};
use super::parse::{parse_meminfo, parse_proc_cgroup, Limite};

const MIB: u64 = 1024 * 1024;
/// Amostras usadas na derivada (as mais recentes).
const JANELA_DERIVADA: usize = 8;

/// Estado e histórico curto de um alvo.
#[derive(Debug)]
struct EstadoAlvo {
    alvo: Alvo,
    amostras: Anel<(u64, u64)>,
    oom_kill: Option<u64>,
    high_cnt: Option<u64>,
    armado_high: bool,
    armado_cresc: bool,
    armado_psi: [bool; 3],
    ultimo_erro: Option<String>,
}

/// Observa o host e N alvos; ver o módulo.
#[derive(Debug)]
pub struct Observador {
    cfg: ConfigMonitor,
    estados: Vec<EstadoAlvo>,
    host_armado: bool,
    host_erro: Option<String>,
}

impl Observador {
    /// **O quê:** observador com `cfg` normalizada e `alvos`. **Onde:** [`super::monitor::Monitor`].
    pub fn novo(cfg: &ConfigMonitor, alvos: Vec<Alvo>) -> Self {
        let cfg = cfg.normalizada();
        let mut o = Observador { cfg, estados: Vec::new(), host_armado: false, host_erro: None };
        o.definir_alvos(alvos);
        o
    }

    /// **O quê:** troca o conjunto de alvos; alvo idêntico mantém estado, novo começa limpo,
    /// removido some. **Onde:** orquestrador admite/encerra unidades.
    pub fn definir_alvos(&mut self, alvos: Vec<Alvo>) {
        let cap = self.cfg.capacidade_anel;
        let mut antigos = std::mem::take(&mut self.estados);
        for a in alvos {
            match antigos.iter().position(|e| e.alvo == a) {
                Some(i) => self.estados.push(antigos.swap_remove(i)),
                None => self.estados.push(EstadoAlvo::novo(a, cap)),
            }
        }
    }

    /// **O quê:** número de alvos. **Onde:** testes/telemetria.
    pub fn n_alvos(&self) -> usize {
        self.estados.len()
    }

    /// **O quê:** uma rodada de leitura; devolve os eventos novos. **Onde:** loop do monitor.
    pub fn amostrar(&mut self, f: &dyn Fonte, agora_ms: u64) -> Vec<Evento> {
        let mut out = Vec::new();
        self.amostrar_host(f, &mut out);
        for e in self.estados.iter_mut() {
            e.amostrar(f, agora_ms, &self.cfg, &mut out);
        }
        out
    }

    /// Lê `MemAvailable` e emite `HostApertado` na borda (rearma a 120% da margem).
    fn amostrar_host(&mut self, f: &dyn Fonte, out: &mut Vec<Evento>) {
        let lido = f.ler("/proc/meminfo").map_err(|e| e.to_string()).and_then(|t| {
            parse_meminfo(&t)?.disponivel.ok_or_else(|| "sem MemAvailable".to_string())
        });
        let disp = match lido {
            Ok(d) => d,
            Err(m) => return erro_borda(&mut self.host_erro, "host", Some(m), out),
        };
        erro_borda(&mut self.host_erro, "host", None, out);
        let margem = self.cfg.margem_host_mib.saturating_mul(MIB);
        if disp < margem && !self.host_armado {
            self.host_armado = true;
            out.push(Evento::HostApertado { disponivel_bytes: disp, margem_bytes: margem });
        } else if u128::from(disp) * 100 >= u128::from(margem) * 120 {
            self.host_armado = false;
        }
    }
}

/// Emite `FalhaLeitura` quando o motivo muda (`None` = recuperou, só limpa).
fn erro_borda(
    ultimo: &mut Option<String>,
    alvo: &str,
    novo: Option<String>,
    out: &mut Vec<Evento>,
) {
    if *ultimo == novo {
        return;
    }
    if let Some(m) = &novo {
        out.push(Evento::FalhaLeitura { alvo: alvo.to_string(), motivo: m.clone() });
    }
    *ultimo = novo;
}

/// Aumento de um contador desde a última amostra; regressão ou 1ª leitura = 0 (reinicia base).
fn delta_contador(prev: &mut Option<u64>, novo: Option<u64>) -> u64 {
    let Some(n) = novo else { return 0 };
    let d = prev.map_or(0, |p| n.saturating_sub(p));
    *prev = Some(n);
    d
}

/// **O quê:** derivada de `memory.current` (MiB/s) nas últimas amostras `(t_ms, bytes)`;
/// `None` com < 3 amostras ou sem avanço de tempo. **Onde:** `CrescimentoRapido` e testes.
pub fn derivada_mib_s(a: &Anel<(u64, u64)>) -> Option<f64> {
    let n = a.len();
    if n < 3 {
        return None;
    }
    let (t0, b0) = a.get(n.saturating_sub(JANELA_DERIVADA))?;
    let (t1, b1) = a.get(n - 1)?;
    if t1 <= t0 {
        return None;
    }
    let delta_mib = (b1 as f64 - b0 as f64) / MIB as f64;
    Some(delta_mib / ((t1 - t0) as f64 / 1000.0))
}

/// Cgroup absoluto de um alvo; `Err` com motivo se não resolve.
fn resolver_dir(f: &dyn Fonte, alvo: &Alvo) -> Result<String, String> {
    let rel = match &alvo.origem {
        Origem::Cgroup(r) => r.clone(),
        Origem::Pid(p) => {
            let t = f.ler(&format!("/proc/{p}/cgroup")).map_err(|e| e.to_string())?;
            parse_proc_cgroup(&t)?
        }
    };
    if rel.split('/').any(|c| c == "..") || rel.contains('\0') {
        return Err(format!("cgroup com '..' recusado: {rel}"));
    }
    Ok(format!("{CGROUP_RAIZ}/{}", rel.trim_start_matches('/')).trim_end_matches('/').to_string())
}

impl EstadoAlvo {
    /// Estado inicial desarmado.
    fn novo(alvo: Alvo, cap: usize) -> Self {
        EstadoAlvo {
            alvo,
            amostras: Anel::novo(cap),
            oom_kill: None,
            high_cnt: None,
            armado_high: false,
            armado_cresc: false,
            armado_psi: [false; 3],
            ultimo_erro: None,
        }
    }

    /// Uma rodada deste alvo: resolve, lê e roda os 4 detectores.
    fn amostrar(&mut self, f: &dyn Fonte, agora: u64, cfg: &ConfigMonitor, out: &mut Vec<Evento>) {
        let dir = match resolver_dir(f, &self.alvo) {
            Ok(d) => d,
            Err(m) => return erro_borda(&mut self.ultimo_erro, &self.alvo.nome, Some(m), out),
        };
        let l = Leitura::de_cgroup_essencial(f, &dir);
        if let Some(m) = l.memoria_atual.motivo.clone() {
            return erro_borda(&mut self.ultimo_erro, &self.alvo.nome, Some(m), out);
        }
        erro_borda(&mut self.ultimo_erro, &self.alvo.nome, None, out);
        self.detectar_high(&l, out);
        self.detectar_oom(&l, out);
        self.detectar_psi(&l, cfg, out);
        self.detectar_crescimento(&l, agora, cfg, out);
    }

    /// `CruzouHigh` na borda; rearma abaixo de 90% do high (ou sem high e sem delta).
    fn detectar_high(&mut self, l: &Leitura, out: &mut Vec<Evento>) {
        let Some(atual) = l.memoria_atual.v() else { return };
        let high = l.memoria_high.v().and_then(Limite::bytes);
        let delta = delta_contador(&mut self.high_cnt, l.eventos.v().and_then(|e| e.high));
        let cruzou = high.is_some_and(|h| atual >= h) || delta > 0;
        if cruzou && !self.armado_high {
            self.armado_high = true;
            out.push(Evento::CruzouHigh {
                alvo: self.alvo.nome.clone(),
                atual_bytes: atual,
                high_bytes: high,
                delta_events_high: delta,
            });
        } else if !cruzou && high.is_none_or(|h| u128::from(atual) * 100 < u128::from(h) * 90) {
            self.armado_high = false;
        }
    }

    /// `OomKillOcorreu` a cada aumento de `oom_kill`.
    fn detectar_oom(&mut self, l: &Leitura, out: &mut Vec<Evento>) {
        let total = l.eventos.v().and_then(|e| e.oom_kill);
        let delta = delta_contador(&mut self.oom_kill, total);
        if let (true, Some(total)) = (delta > 0, total) {
            out.push(Evento::OomKillOcorreu { alvo: self.alvo.nome.clone(), delta, total });
        }
    }

    /// `PressaoPsi` quando `full avg10` passa do limiar; rearma abaixo da metade.
    fn detectar_psi(&mut self, l: &Leitura, cfg: &ConfigMonitor, out: &mut Vec<Evento>) {
        let rec = [
            (Recurso::Memoria, &l.psi_memoria),
            (Recurso::Cpu, &l.psi_cpu),
            (Recurso::Io, &l.psi_io),
        ];
        for (i, (recurso, campo)) in rec.into_iter().enumerate() {
            let Some(x) = campo.v().and_then(|p| p.full).map(|f| f.avg10) else { continue };
            if x > cfg.psi_full_limiar && !self.armado_psi[i] {
                self.armado_psi[i] = true;
                out.push(Evento::PressaoPsi {
                    alvo: self.alvo.nome.clone(),
                    recurso,
                    full_avg10: x,
                    limiar: cfg.psi_full_limiar,
                });
            } else if x < cfg.psi_full_limiar / 2.0 {
                self.armado_psi[i] = false;
            }
        }
    }

    /// `CrescimentoRapido` com projeção até `memory.max`; rearma abaixo da metade do limiar.
    fn detectar_crescimento(
        &mut self,
        l: &Leitura,
        agora: u64,
        cfg: &ConfigMonitor,
        out: &mut Vec<Evento>,
    ) {
        let Some(atual) = l.memoria_atual.v() else { return };
        if self.amostras.ultimo().is_some_and(|(t, _)| agora <= t) {
            self.amostras.limpar();
        }
        self.amostras.push((agora, atual));
        let Some(mib_s) = derivada_mib_s(&self.amostras) else { return };
        if mib_s > cfg.crescimento_mib_s && !self.armado_cresc {
            self.armado_cresc = true;
            let teto = l.memoria_max.v().and_then(Limite::bytes);
            let secs = teto.map(|t| match t.checked_sub(atual) {
                Some(resta) if resta > 0 => resta as f64 / MIB as f64 / mib_s,
                _ => 0.0,
            });
            out.push(Evento::CrescimentoRapido {
                alvo: self.alvo.nome.clone(),
                mib_por_s: mib_s,
                atual_bytes: atual,
                teto_bytes: teto,
                segundos_ate_teto: secs,
            });
        } else if mib_s < cfg.crescimento_mib_s / 2.0 {
            self.armado_cresc = false;
        }
    }
}
