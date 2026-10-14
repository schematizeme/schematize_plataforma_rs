//! De qual commit este binário foi construído — e a decisão de como dizer isso.
//!
//! **O quê:** a decisão pura ([`rotulo`], [`sha_valido`], [`sujo_de`]) e o macro
//! [`rotulo_versao!`](crate::rotulo_versao) que cola as variáveis do `build.rs` DO HOST.
//!
//! **Onde:** o `--version` de cada binário (clap `#[command(version = ...)]` ou a janela).
//!
//! ## Por que um macro
//!
//! `env!("SCHEMATIZE_GIT_SHA")` é resolvido na compilação do crate que o escreve; dentro da lib
//! leria o ambiente da lib, não o do app. O macro expande no host, que passa o NOME das
//! variáveis (cada repo usa as suas: `SCHEMATIZE_GIT_*`, `SCHEMATIZE_OVERDEV_*`, `..._GUI_*`).
//!
//! ```ignore
//! // build.rs do host emite SCHEMATIZE_GIT_SHA e SCHEMATIZE_GIT_DIRTY ("1" = sujo)
//! let v: &'static str = schematize_plataforma::rotulo_versao!("SCHEMATIZE_GIT_SHA", "SCHEMATIZE_GIT_DIRTY");
//! ```

/// **O quê:** o SHA saneado (sem espaços nas pontas) se for hexadecimal de 7 a 40 dígitos;
/// `None` caso contrário (vazio, lixo, `fatal:` de erro de git no stdout).
///
/// **Onde:** [`rotulo`] e o `doctor`/guard que compara o binário com o HEAD do repo.
pub fn sha_valido(sha: &str) -> Option<&str> {
    let limpo = sha.trim();
    ((7..=40).contains(&limpo.len()) && limpo.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(limpo)
}

/// **O quê:** `true` quando o valor do `..._DIRTY` do build é `"1"` (com espaços tolerados).
///
/// **Onde:** o macro [`rotulo_versao!`](crate::rotulo_versao) e os hosts que só querem a marca.
pub fn sujo_de(valor: &str) -> bool {
    valor.trim() == "1"
}

/// **O quê:** `"0.2.1 (b9fcc65abcde)"`; `"0.2.1 (b9fcc65abcde-sujo)"` se o build tinha mudança
/// não commitada (o SHA sozinho mentiria); `"0.2.1 (fonte sem git)"` quando o SHA não veio
/// (sem SHA, `sujo` é irrelevante).
///
/// **Onde:** o macro [`rotulo_versao!`](crate::rotulo_versao) e os testes.
///
/// **Por que nunca esconder o caso vazio:** um `--version` que mostra só o número quando não
/// sabe o commit é indistinguível de um binário antigo que nunca soube.
///
/// **Por que SHA malformado conta como ausente:** se o `build.rs` emitir lixo, mostrar o lixo
/// como se fosse procedência é pior que dizer que não sabe.
pub fn rotulo(versao: &str, sha: &str, sujo: bool) -> String {
    match sha_valido(sha) {
        Some(s) => format!("{versao} ({s}{})", if sujo { "-sujo" } else { "" }),
        None => format!("{versao} (fonte sem git)"),
    }
}

/// **O quê:** o rótulo de versão do HOST, `&'static str` (o clap exige `Str`), calculado uma
/// vez por `OnceLock`. Args: NOMES (literais) das variáveis de build do SHA e do DIRTY.
/// A versão vem do `CARGO_PKG_VERSION` do host.
///
/// **Onde:** `#[command(version = ...)]` ou o `--version` da janela de cada app. Falha de
/// compilação (e não `panic` em runtime) se o `build.rs` do host não emitir as variáveis.
#[macro_export]
macro_rules! rotulo_versao {
    ($sha:literal, $sujo:literal) => {{
        static ROTULO: ::std::sync::OnceLock<::std::string::String> = ::std::sync::OnceLock::new();
        ROTULO
            .get_or_init(|| {
                $crate::procedencia::rotulo(
                    env!("CARGO_PKG_VERSION"),
                    env!($sha),
                    $crate::procedencia::sujo_de(env!($sujo)),
                )
            })
            .as_str()
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA de verdade entra no rótulo como veio.
    #[test]
    fn sha_valido_aparece_no_rotulo() {
        assert_eq!(rotulo("0.2.1", "b9fcc65abcde", false), "0.2.1 (b9fcc65abcde)");
        assert_eq!(rotulo("1.0.0", "abcdef0", false), "1.0.0 (abcdef0)", "7 dígitos é o mínimo");
        let quarenta = "a".repeat(40);
        assert_eq!(rotulo("1", &quarenta, false), format!("1 ({quarenta})"));
        assert_eq!(rotulo("1", " b9fcc65 \n", false), "1 (b9fcc65)", "espaços das pontas saem");
    }

    /// **O caso do tarball:** sem git, o build NÃO falha e a versão diz o que houve.
    #[test]
    fn sem_git_o_rotulo_diz_que_nao_sabe_em_vez_de_omitir() {
        assert_eq!(rotulo("0.2.1", "", false), "0.2.1 (fonte sem git)");
        assert_eq!(rotulo("0.2.1", "   ", false), "0.2.1 (fonte sem git)");
    }

    /// Lixo NÃO passa por procedência — senão alguém compara com o HEAD e tira conclusão.
    #[test]
    fn sha_malformado_conta_como_ausente() {
        for ruim in ["fatal:", "nao-hex-aqui", "abc", "zzzzzzzzz", "b9fcc65 extra"] {
            assert_eq!(rotulo("0.2.1", ruim, false), "0.2.1 (fonte sem git)", "{ruim:?}");
        }
    }

    /// Hostis: tamanho de fronteira, unicode, byte nulo, hex fullwidth, SHA longo demais.
    #[test]
    fn sha_hostil_unicode_nulo_e_fronteiras() {
        let seis = "abcdef";
        let quarenta_e_um = "a".repeat(41);
        let fullwidth = "ａｂｃｄｅｆ０１２"; // dígitos hex "largos" não são ASCII
        for ruim in [
            seis,
            quarenta_e_um.as_str(),
            fullwidth,
            "b9fcc65\0",
            "日本語日本語日本語",
            "0x1234567",
        ] {
            assert_eq!(sha_valido(ruim), None, "{ruim:?}");
            assert_eq!(rotulo("v", ruim, true), "v (fonte sem git)", "{ruim:?}");
        }
        assert_eq!(sha_valido("ABCDEF0123"), Some("ABCDEF0123"), "maiúsculas são hex");
    }

    /// **Árvore suja é dita:** o SHA é o do HEAD, mas o binário tem mais que o HEAD.
    #[test]
    fn arvore_suja_marca_o_sha_e_limpa_nao() {
        assert_eq!(rotulo("0.2.1", "b9fcc65abcde", true), "0.2.1 (b9fcc65abcde-sujo)");
        assert_eq!(rotulo("0.2.1", "b9fcc65abcde", false), "0.2.1 (b9fcc65abcde)");
    }

    /// Sem SHA válido não há o que marcar: continua "fonte sem git", suja ou não.
    #[test]
    fn sujo_sem_sha_continua_fonte_sem_git() {
        assert_eq!(rotulo("0.2.1", "", true), "0.2.1 (fonte sem git)");
        assert_eq!(rotulo("0.2.1", "lixo", true), "0.2.1 (fonte sem git)");
    }

    /// Só `"1"` (com espaços) é sujo; o resto não.
    #[test]
    fn sujo_de_so_aceita_um() {
        assert!(sujo_de("1") && sujo_de(" 1\n"));
        for n in ["", "0", "true", "11", "sim"] {
            assert!(!sujo_de(n), "{n:?}");
        }
    }

    /// O macro lê as variáveis DO HOST (aqui, este crate): `CARGO_PKG_NAME` não é hex ->
    /// "fonte sem git"; e a versão vem do `CARGO_PKG_VERSION`.
    #[test]
    fn macro_expande_no_host_e_e_estavel() {
        let a: &'static str = crate::rotulo_versao!("CARGO_PKG_NAME", "CARGO_PKG_NAME");
        assert_eq!(a, format!("{} (fonte sem git)", env!("CARGO_PKG_VERSION")));
        let b = crate::rotulo_versao!("CARGO_PKG_NAME", "CARGO_PKG_NAME");
        assert_eq!(a, b, "duas expansões dão o mesmo rótulo");
    }
}
