//! `Maquina<E>`: máquina de estados durável sobre journal + snapshot, com eventos idempotentes.
//!
//! `estado = snapshot + replay do journal depois do seq do snapshot`. O overdev pluga o seu
//! modelo implementando [`Estado`]; esta camada só entrega a infraestrutura.

use super::erro::Erro;
use super::journal::{Config, Journal, Relatorio};
use super::snapshot::{Origem, Snapshot};
use std::path::{Path, PathBuf};

/// Tipo de registro usado para eventos da máquina.
const TIPO_EVENTO: u8 = 1;

/// **O quê:** contrato do modelo de estado plugável.
///
/// **Onde:** implementado pelo overdev (itens do checklist, unidade em voo, tentativas,
/// geração de re-decomposição, modelo usado). `aplicar` deve ser determinístico; a
/// idempotência por seq é garantida pela [`Maquina`] (reaplicar o mesmo seq é no-op).
pub trait Estado: Sized {
    /// Evento do domínio.
    type Evento;
    /// Versão do esquema do snapshot; mudou o formato, mude este número.
    const VERSAO_ESQUEMA: u32;
    /// Estado vazio (antes de qualquer evento).
    fn inicial() -> Self;
    /// Aplica um evento (determinístico, sem I/O).
    fn aplicar(&mut self, evento: &Self::Evento);
    /// Serializa o estado inteiro (snapshot).
    fn codificar(&self) -> Vec<u8>;
    /// Reconstrói o estado do snapshot.
    fn decodificar(bytes: &[u8]) -> Result<Self, String>;
    /// Serializa um evento (payload do journal).
    fn codificar_evento(evento: &Self::Evento) -> Vec<u8>;
    /// Reconstrói um evento.
    fn decodificar_evento(bytes: &[u8]) -> Result<Self::Evento, String>;
}

/// **O quê:** estado durável: journal + snapshot + estado em memória.
///
/// **Onde:** núcleo de retomada do orquestrador.
pub struct Maquina<E: Estado> {
    dir: PathBuf,
    journal: Journal,
    estado: E,
    seq_aplicado: u64,
    seq_ultimo_snapshot: u64,
}

impl<E: Estado> Maquina<E> {
    /// **O quê:** abre o diretório, carrega o snapshot (cai no `.prev` se o último estiver
    /// ruim), recupera o journal e faz replay dos eventos posteriores.
    ///
    /// **Onde:** boot (inclusive `--resume`). Snapshot inutilizável só é tolerado se o journal
    /// ainda começa no seq 1 (replay do zero); senão o erro sobe.
    pub fn abrir(dir: &Path, cfg: Config) -> Result<(Maquina<E>, Relatorio), Erro> {
        let mut journal = Journal::abrir(dir, cfg)?;
        let carregado = Snapshot::carregar(dir);
        let (regs, rel) = journal.recuperar()?;
        let (mut estado, base, origem, aviso) =
            base_do_estado::<E>(carregado, regs.first().map(|r| r.seq))?;
        let mut rel = rel;
        rel.snapshot_aviso = aviso;
        let mut m_seq = base;
        for r in regs.iter().filter(|r| r.seq > base) {
            if r.seq != m_seq + 1 {
                let segmento = dir.to_path_buf();
                return Err(Erro::BuracoDeSeq { esperado: m_seq + 1, encontrado: r.seq, segmento });
            }
            aplica_registro::<E>(&mut estado, r.seq, r.tipo, &r.payload)?;
            m_seq = r.seq;
        }
        if journal.proximo_seq() <= m_seq {
            journal.pular_para(m_seq + 1)?;
        }
        let seq_ultimo_snapshot = if origem.is_some() { base } else { 0 };
        let m = Maquina {
            dir: dir.to_path_buf(),
            journal,
            estado,
            seq_aplicado: m_seq,
            seq_ultimo_snapshot,
        };
        Ok((m, rel))
    }

    /// **O quê:** estado atual (somente leitura). **Onde:** decisões do orquestrador.
    pub fn estado(&self) -> &E {
        &self.estado
    }

    /// **O quê:** seq do último evento aplicado. **Onde:** diagnóstico e testes.
    pub fn seq(&self) -> u64 {
        self.seq_aplicado
    }

    /// **O quê:** registra o evento NO JOURNAL primeiro (write-ahead) e só então o aplica.
    ///
    /// **Onde:** cada transição de estado. Se o anexo falha, o estado em memória não muda.
    pub fn registrar(&mut self, evento: &E::Evento) -> Result<u64, Erro> {
        let seq = self.journal.anexar(TIPO_EVENTO, &E::codificar_evento(evento))?;
        self.aplicar_seq(seq, evento);
        Ok(seq)
    }

    /// **O quê:** aplica `evento` somente se `seq` for novo; reaplicar é no-op (devolve `false`).
    ///
    /// **Onde:** replay e `registrar`. É a garantia de idempotência.
    pub fn aplicar_seq(&mut self, seq: u64, evento: &E::Evento) -> bool {
        if seq <= self.seq_aplicado {
            return false;
        }
        self.estado.aplicar(evento);
        self.seq_aplicado = seq;
        true
    }

    /// **O quê:** força `fsync` do journal. **Onde:** com `Durabilidade::Lote`/`Nunca`.
    pub fn sincronizar(&mut self) -> Result<(), Erro> {
        self.journal.sincronizar()
    }

    /// **O quê:** grava snapshot do estado atual e compacta o journal até o seq do snapshot
    /// ANTERIOR (o `.prev` precisa continuar recuperável por replay).
    ///
    /// **Onde:** periodicamente / a cada N eventos / no shutdown.
    pub fn snapshot(&mut self) -> Result<(), Erro> {
        self.journal.sincronizar()?;
        let snap = Snapshot {
            versao_esquema: E::VERSAO_ESQUEMA,
            seq: self.seq_aplicado,
            dados: self.estado.codificar(),
        };
        snap.gravar(&self.dir)?;
        self.journal.compactar(self.seq_ultimo_snapshot)?;
        self.seq_ultimo_snapshot = snap.seq;
        Ok(())
    }
}

/// **O quê:** decide o estado-base (snapshot ou inicial) e o seq dele.
///
/// **Onde:** `Maquina::abrir`. Snapshot inválido só é tolerado com journal começando em 1 (o
/// motivo vai no aviso do relatório); caso contrário o erro sobe.
fn base_do_estado<E: Estado>(
    carregado: Result<Option<(Snapshot, Origem)>, Erro>,
    primeiro_seq: Option<u64>,
) -> Result<(E, u64, Option<Origem>, Option<String>), Erro> {
    match carregado {
        Ok(Some((s, o))) => {
            if s.versao_esquema != E::VERSAO_ESQUEMA {
                let (snapshot, esperado) = (s.versao_esquema, E::VERSAO_ESQUEMA);
                return Err(Erro::EsquemaIncompativel { snapshot, esperado });
            }
            let e = E::decodificar(&s.dados).map_err(Erro::SnapshotInvalido)?;
            let aviso =
                (o == Origem::Anterior).then(|| "snapshot atual ruim: usei o .prev".to_string());
            Ok((e, s.seq, Some(o), aviso))
        }
        Ok(None) => Ok((E::inicial(), 0, None, None)),
        Err(e) if primeiro_seq == Some(1) => {
            Ok((E::inicial(), 0, None, Some(format!("snapshot descartado, replay do zero: {e}"))))
        }
        Err(e) => Err(e),
    }
}

/// **O quê:** decodifica e aplica um registro do journal.
///
/// **Onde:** replay em `abrir`. Tipo desconhecido ou evento ilegível = erro explícito.
fn aplica_registro<E: Estado>(
    estado: &mut E,
    seq: u64,
    tipo: u8,
    payload: &[u8],
) -> Result<(), Erro> {
    if tipo != TIPO_EVENTO {
        return Err(Erro::EventoInvalido { seq, motivo: format!("tipo {tipo} desconhecido") });
    }
    let ev =
        E::decodificar_evento(payload).map_err(|motivo| Erro::EventoInvalido { seq, motivo })?;
    estado.aplicar(&ev);
    Ok(())
}
