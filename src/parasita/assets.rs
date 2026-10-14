//! Decisões puras do parasita sobre ONDE estão os binários e DE ONDE baixá-los.
//!
//! **O quê:** nomes dos dois binários do market, nome do arquivo por plataforma, URL do asset
//! publicado e o mapeamento `aarch64 -> arm64`.
//!
//! **Onde:** `parasita::instalar` e `parasita::garantir_em`. Nada aqui faz I/O além de `stat`.

use std::path::Path;

/// Os dois binários do gestor, na ordem em que são garantidos.
pub const NOMES: [&str; 2] = ["schematize-market", "schematize-market-gui"];

/// Base dos assets do release `latest` do market.
pub const BASE_URL: &str =
    "https://github.com/schematizeme/schematize_market_rs/releases/latest/download";

/// Comando de instalação manual citado na mensagem de falha.
pub const INSTALAR: &str =
    "curl -fsSL https://raw.githubusercontent.com/schematizeme/schematize-cli/main/install.sh | bash";

/// **O quê:** o nome do arquivo no disco para `os` (`.exe` no Windows).
///
/// **Onde:** [`o_que_falta_para`] e a montagem do destino em `garantir_em`.
pub fn nome_do_arquivo(nome: &str, os: &str) -> String {
    if os == "windows" {
        format!("{nome}.exe")
    } else {
        nome.to_string()
    }
}

/// **O quê:** quais dos dois binários NÃO existem em `pasta` para o `os` dado (decisão pura:
/// só `stat`).
///
/// **Onde:** [`o_que_falta`] e os testes (que simulam outro `os`).
pub fn o_que_falta_para(pasta: &Path, os: &str) -> Vec<&'static str> {
    NOMES.iter().copied().filter(|n| !pasta.join(nome_do_arquivo(n, os)).is_file()).collect()
}

/// **O quê:** quais dos dois binários NÃO existem em `pasta` no SO atual.
///
/// **Onde:** `garantir_em`, no caminho rápido (2 `stat`, sem rede).
pub fn o_que_falta(pasta: &Path) -> Vec<&'static str> {
    o_que_falta_para(pasta, std::env::consts::OS)
}

/// **O quê:** normaliza a arquitetura para a nomenclatura dos assets (`aarch64` e `arm64`
/// viram `arm64`; `x86_64` fica); `None` se não é uma que o market publica.
///
/// **Onde:** [`url_do_asset`] e [`plataforma_atual`].
fn arch_do_asset(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("arm64"),
        _ => None,
    }
}

/// **O quê:** a URL do asset de `nome` para `os`/`arch` (como em `std::env::consts`), ou `None`
/// quando a plataforma não tem asset publicado. Publicados: linux/freebsd x86_64, macos
/// x86_64+arm64 e windows x86_64 (`.exe`).
///
/// **Onde:** `garantir_em`, uma vez por binário que falta.
pub fn url_do_asset(nome: &str, os: &str, arch: &str) -> Option<String> {
    let arch = arch_do_asset(arch)?;
    let sufixo = match (os, arch) {
        ("linux" | "freebsd", "x86_64") | ("macos", "x86_64" | "arm64") => "",
        ("windows", "x86_64") => ".exe",
        _ => return None,
    };
    Some(format!("{BASE_URL}/{nome}-{os}-{arch}{sufixo}"))
}

/// **O quê:** `(os, arch)` desta máquina na nomenclatura dos assets (`aarch64` vira `arm64`).
/// Arquitetura desconhecida passa crua (e `url_do_asset` devolverá `None`).
///
/// **Onde:** `garantir_com`.
pub fn plataforma_atual() -> (&'static str, &'static str) {
    let arch = std::env::consts::ARCH;
    (std::env::consts::OS, arch_do_asset(arch).unwrap_or(arch))
}
