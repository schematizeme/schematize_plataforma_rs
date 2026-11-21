//! [`Monitor`]: thread única que amostra N alvos e emite [`Evento`] por canal limitado.
//!
//! BUDGET de memória do próprio módulo (provado por `tests/memoria_budget.rs`, VmHWM com 64
//! alvos): < 2 MiB de RSS adicional. Composição: anel `32 x 16 B` por alvo (~0,5 KiB) + estado
//! (~200 B) + canal `capacidade_canal x ~120 B` (default 256 = ~30 KiB) + pilha do thread
//! (256 KiB reservados, só as páginas tocadas contam) + buffers de leitura transitórios
//! (<= 256 KiB por arquivo, liberados a cada leitura). Nada cresce com o tempo.
//!
//! **Onde:** orquestrador que roda muitas unidades jauladas; consome o `Receiver<Evento>`.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::evento::{Alvo, ConfigMonitor, Evento};
use super::fonte::Fonte;
use super::observador::Observador;

/// Pilha do thread do monitor.
const PILHA_BYTES: usize = 256 * 1024;

/// Falha ao iniciar/encerrar o monitor.
#[derive(Debug)]
pub enum ErroMonitor {
    /// Não foi possível criar o thread.
    Spawn(String),
    /// O thread do monitor entrou em panic.
    Panico,
}

impl fmt::Display for ErroMonitor {
    /// **O quê:** mensagem acionável. **Onde:** log de quem inicia/para o monitor.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroMonitor::Spawn(m) => write!(f, "não consegui criar o thread do monitor: {m}"),
            ErroMonitor::Panico => write!(f, "o thread do monitor terminou em panic"),
        }
    }
}

impl std::error::Error for ErroMonitor {}

/// Handle do thread de monitoramento. `Drop` para e junta o thread.
#[derive(Debug)]
pub struct Monitor {
    parar: Arc<AtomicBool>,
    descartados: Arc<AtomicU64>,
    pendentes: Arc<Mutex<Option<Vec<Alvo>>>>,
    th: Option<JoinHandle<()>>,
}

impl Monitor {
    /// **O quê:** sobe o thread (intervalo clampado em 250 ms–5 s) e devolve o handle e o canal.
    /// Canal cheio descarta o evento e incrementa [`Self::descartados`]; nunca bloqueia.
    /// **Onde:** início do orquestrador (ou de cada lote de unidades).
    pub fn iniciar(
        cfg: &ConfigMonitor,
        fonte: Arc<dyn Fonte>,
        alvos: Vec<Alvo>,
    ) -> Result<(Monitor, Receiver<Evento>), ErroMonitor> {
        let cfg = cfg.normalizada();
        let (tx, rx) = sync_channel(cfg.capacidade_canal);
        let parar = Arc::new(AtomicBool::new(false));
        let descartados = Arc::new(AtomicU64::new(0));
        let pendentes = Arc::new(Mutex::new(None));
        let obs = Observador::novo(&cfg, alvos);
        let ctx = Contexto {
            obs,
            fonte,
            intervalo: cfg.intervalo,
            parar: Arc::clone(&parar),
            descartados: Arc::clone(&descartados),
            pendentes: Arc::clone(&pendentes),
        };
        let th = thread::Builder::new()
            .name("memoria-monitor".to_string())
            .stack_size(PILHA_BYTES)
            .spawn(move || ctx.laco(tx))
            .map_err(|e| ErroMonitor::Spawn(e.to_string()))?;
        Ok((Monitor { parar, descartados, pendentes, th: Some(th) }, rx))
    }

    /// **O quê:** troca os alvos observados (aplicada na próxima amostra). **Onde:** o
    /// orquestrador admite/encerra unidades.
    pub fn definir_alvos(&self, alvos: Vec<Alvo>) {
        let mut g = self.pendentes.lock().unwrap_or_else(PoisonError::into_inner);
        *g = Some(alvos);
    }

    /// **O quê:** eventos perdidos por canal cheio. **Onde:** alarme "consumidor lento".
    pub fn descartados(&self) -> u64 {
        self.descartados.load(Ordering::Relaxed)
    }

    /// **O quê:** para e junta o thread; `Err(Panico)` se ele havia caído. **Onde:** shutdown.
    pub fn parar(mut self) -> Result<(), ErroMonitor> {
        self.encerrar()
    }

    /// Sinaliza, acorda e junta o thread (idempotente).
    fn encerrar(&mut self) -> Result<(), ErroMonitor> {
        self.parar.store(true, Ordering::SeqCst);
        match self.th.take() {
            Some(th) => {
                th.thread().unpark();
                th.join().map_err(|_| ErroMonitor::Panico)
            }
            None => Ok(()),
        }
    }
}

impl Drop for Monitor {
    /// **O quê:** encerra o thread; panic anterior vai para stderr (nunca silencioso).
    /// **Onde:** fim de escopo do handle.
    fn drop(&mut self) {
        if let Err(e) = self.encerrar() {
            eprintln!("aviso: {e}");
        }
    }
}

/// Tudo que o thread precisa (movido para dentro dele).
struct Contexto {
    obs: Observador,
    fonte: Arc<dyn Fonte>,
    intervalo: Duration,
    parar: Arc<AtomicBool>,
    descartados: Arc<AtomicU64>,
    pendentes: Arc<Mutex<Option<Vec<Alvo>>>>,
}

impl Contexto {
    /// Laço: aplica alvos novos, amostra, entrega com `try_send`, dorme até o próximo tick.
    fn laco(mut self, tx: std::sync::mpsc::SyncSender<Evento>) {
        let t0 = Instant::now();
        while !self.parar.load(Ordering::SeqCst) {
            self.aplicar_pendentes();
            let ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
            for ev in self.obs.amostrar(&*self.fonte, ms) {
                match tx.try_send(ev) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        self.descartados.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
            self.dormir();
        }
    }

    /// Aplica a lista de alvos pendente, se houver.
    fn aplicar_pendentes(&mut self) {
        let novos = self.pendentes.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(a) = novos {
            self.obs.definir_alvos(a);
        }
    }

    /// Dorme até o prazo, acordando cedo só se `parar` for sinalizado.
    fn dormir(&self) {
        let prazo = Instant::now() + self.intervalo;
        while !self.parar.load(Ordering::SeqCst) {
            let agora = Instant::now();
            if agora >= prazo {
                break;
            }
            thread::park_timeout(prazo - agora);
        }
    }
}
