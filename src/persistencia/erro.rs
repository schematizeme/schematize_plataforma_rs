//! Erro único da persistência. Nada aqui é engolido: toda falha de I/O carrega o contexto.

use std::fmt;
use std::path::PathBuf;

/// **O quê:** todas as falhas possíveis do journal, do snapshot, da máquina e do vigia.
///
/// **Onde:** retorno de toda função pública do módulo `persistencia`.
#[derive(Debug)]
pub enum Erro {
    /// Falha de I/O, com o que se tentava fazer.
    Io { contexto: String, fonte: std::io::Error },
    /// Outro escritor já detém o lock exclusivo (`pid` quando legível no arquivo de lock).
    LockOcupado { dir: PathBuf, pid: Option<u32> },
    /// Registro válido (CRC ok) com seq diferente do esperado: buraco ou duplicata.
    BuracoDeSeq { esperado: u64, encontrado: u64, segmento: PathBuf },
    /// Corrupção em segmento que não é o último: não é torn write, não truncamos em silêncio.
    CorrupcaoNoMeio { segmento: PathBuf, offset: u64, motivo: String },
    /// `anexar` antes de `recuperar`.
    NaoRecuperado,
    /// O journal ficou em estado incerto (fsync ou rollback falhou): reabra e recupere.
    Envenenado(String),
    /// Payload acima do teto.
    PayloadGrande { tam: usize, max: usize },
    /// Snapshot ilegível, com checksum errado ou truncado.
    SnapshotInvalido(String),
    /// Snapshot gravado por outra versão de esquema.
    EsquemaIncompativel { snapshot: u32, esperado: u32 },
    /// Evento que o modelo não soube decodificar.
    EventoInvalido { seq: u64, motivo: String },
    /// Configuração inválida.
    Config(String),
}

impl fmt::Display for Erro {
    /// **O quê:** mensagem legível e acionável. **Onde:** logs e mensagens ao usuário.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erro::Io { contexto, fonte } => write!(f, "{contexto}: {fonte}"),
            Erro::LockOcupado { dir, pid } => match pid {
                Some(p) => {
                    write!(f, "{}: já há um escritor (pid {p}); só um é permitido", dir.display())
                }
                None => write!(f, "{}: já há um escritor; só um é permitido", dir.display()),
            },
            Erro::BuracoDeSeq { esperado, encontrado, segmento } => write!(
                f,
                "{}: seq esperado {esperado}, encontrado {encontrado} (buraco ou duplicata)",
                segmento.display()
            ),
            Erro::CorrupcaoNoMeio { segmento, offset, motivo } => {
                write!(
                    f,
                    "{}: corrupção no offset {offset} de segmento não-final: {motivo}",
                    segmento.display()
                )
            }
            Erro::NaoRecuperado => {
                write!(f, "journal aberto mas não recuperado: chame recuperar()")
            }
            Erro::Envenenado(m) => write!(f, "journal em estado incerto, reabra e recupere: {m}"),
            Erro::PayloadGrande { tam, max } => {
                write!(f, "payload de {tam} bytes excede o teto de {max}")
            }
            Erro::SnapshotInvalido(m) => write!(f, "snapshot inválido: {m}"),
            Erro::EsquemaIncompativel { snapshot, esperado } => {
                write!(f, "snapshot de esquema v{snapshot}, o código espera v{esperado}")
            }
            Erro::EventoInvalido { seq, motivo } => {
                write!(f, "evento seq {seq} inválido: {motivo}")
            }
            Erro::Config(m) => write!(f, "configuração inválida: {m}"),
        }
    }
}

impl std::error::Error for Erro {
    /// **O quê:** expõe o erro de I/O de origem. **Onde:** cadeias `source()` de quem loga.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Erro::Io { fonte, .. } => Some(fonte),
            _ => None,
        }
    }
}

/// **O quê:** adaptador `map_err` que anexa o contexto a um `io::Error`.
///
/// **Onde:** toda chamada de I/O deste módulo: `.map_err(io("abrir X"))`.
pub fn io(contexto: impl Into<String>) -> impl FnOnce(std::io::Error) -> Erro {
    let contexto = contexto.into();
    move |fonte| Erro::Io { contexto, fonte }
}
