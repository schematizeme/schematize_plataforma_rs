//! Erro tipado do módulo `sistema`: toda leitura que falha vira um destes valores
//! (nunca panic, nunca engolido) e é registrada como motivo no campo [`crate::sistema::Lido`].

use std::fmt;

/// O quê: motivo pelo qual uma leitura/execução/parse não produziu valor.
/// Onde: retornado por [`crate::sistema::Fonte`] e pelos parsers; convertido em `Lido::motivo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroSistema {
    /// Arquivo, diretório ou comando inexistente neste host.
    Ausente(String),
    /// Existe mas o usuário atual não pode ler/executar (sem sudo, não tentamos elevar).
    Permissao(String),
    /// Outra falha de E/S.
    Io(String),
    /// Comando excedeu o tempo limite e foi encerrado.
    Timeout(String),
    /// Conteúdo lido não tem o formato esperado (truncado, hostil, não numérico...).
    Invalido { campo: String, detalhe: String },
}

impl ErroSistema {
    /// O quê: atalho para construir [`ErroSistema::Invalido`].
    /// Onde: usado por todos os parsers de `sistema::parse`.
    pub fn invalido(campo: &str, detalhe: &str) -> Self {
        ErroSistema::Invalido { campo: campo.to_string(), detalhe: detalhe.to_string() }
    }

    /// O quê: converte um `std::io::Error` em variante tipada, anotando o alvo.
    /// Onde: `FonteReal` ao ler arquivos/diretórios/comandos.
    pub fn de_io(alvo: &str, e: &std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => ErroSistema::Ausente(alvo.to_string()),
            std::io::ErrorKind::PermissionDenied => ErroSistema::Permissao(alvo.to_string()),
            _ => ErroSistema::Io(format!("{alvo}: {e}")),
        }
    }
}

impl fmt::Display for ErroSistema {
    /// O quê: texto curto e sem culpa do erro. Onde: `Lido::motivo` e resumo/JSON.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroSistema::Ausente(a) => write!(f, "ausente: {a}"),
            ErroSistema::Permissao(a) => write!(f, "sem permissão de leitura: {a}"),
            ErroSistema::Io(a) => write!(f, "erro de E/S: {a}"),
            ErroSistema::Timeout(a) => write!(f, "tempo esgotado: {a}"),
            ErroSistema::Invalido { campo, detalhe } => write!(f, "inválido em {campo}: {detalhe}"),
        }
    }
}

impl std::error::Error for ErroSistema {}
