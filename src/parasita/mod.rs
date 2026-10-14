//! **market parasita** — todo binário do ecossistema garante, ao iniciar, que o gestor de
//! atualizações (`schematize-market` e `schematize-market-gui`) existe em `$HOME/.cargo/bin`.
//!
//! **O quê:** se faltar algum, baixa o binário da plataforma (`curl`), valida (`--version`
//! sai 0 em até 10 s) e só então move para o destino. Caminho rápido: dois `stat`, sem rede.
//!
//! **Onde:** a primeira coisa do `main` de cada binário (CLI e janela), depois do parse de
//! argumentos: `parasita::garantir(&Config::new("meu-app"))`. Contrato:
//! `.schematize/scratch/market-parasita/CONTRATO.md`.
//!
//! **Nunca derruba o app (piso 10):** toda falha vira linha no stderr e o app segue; uma
//! falha por binário não impede o seguinte. **Nunca engole erro (piso 4):** o que não pôde ser
//! dito vai em [`Resultado::avisos`].
//!
//! **Hooks:** quem tem hooks do Claude Code (rodam a cada tool call e não podem ganhar rede
//! nem latência) declara os padrões em [`Config::hooks`]; o parasita não roda neles.
//!
//! **Testes do host:** [`garantir`] só é inerte sob o `cfg(test)` DESTA lib. O host deve
//! chamá-la do `main` (que os testes dele não executam) ou protegê-la com `#[cfg(not(test))]`.

mod assets;
mod instalar;

pub use assets::{
    nome_do_arquivo, o_que_falta, o_que_falta_para, plataforma_atual, url_do_asset, BASE_URL,
    INSTALAR, NOMES,
};
pub use instalar::{
    baixar_com_curl, baixar_com_programa, caminho_novo, instalar_um, repetir_em_etxtbsy,
    tomar_lock, validar, Baixador, Lock, LOCK_VELHO, VERSAO_TIMEOUT,
};

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
mod testes;
#[cfg(test)]
mod testes_decisao;

/// Configuração que o host preenche.
#[derive(Debug, Clone, Copy)]
pub struct Config<'a> {
    /// Nome do app, usado como prefixo das mensagens (`"<app>: instalei o gestor ..."`).
    pub app: &'a str,
    /// Padrões de subcomando que são HOOKS (o parasita não roda neles). Cada padrão é um
    /// prefixo dos args; cada elemento aceita alternativas com `|`. Ex.: `&["overdev",
    /// "guard|check"]` casa `overdev guard` e `overdev check`; `&["guard|check|hooks"]` casa
    /// `guard ...`. Padrão vazio é ignorado (casaria tudo).
    pub hooks: &'a [&'a [&'a str]],
    /// Teto do `--version` do binário baixado.
    pub versao_timeout: Duration,
}

impl<'a> Config<'a> {
    /// **O quê:** config mínima: só o nome do app, sem hooks, teto de 10 s.
    ///
    /// **Onde:** o `main` de apps sem hooks (optimizer, deployer, GUIs, ...).
    pub const fn new(app: &'a str) -> Self {
        Config { app, hooks: &[], versao_timeout: VERSAO_TIMEOUT }
    }

    /// **O quê:** devolve a config com os padrões de hook do host.
    ///
    /// **Onde:** o `main` de quem tem hooks (cli, overdev).
    pub const fn com_hooks(mut self, hooks: &'a [&'a [&'a str]]) -> Self {
        self.hooks = hooks;
        self
    }

    /// **O quê:** devolve a config com outro teto para o `--version` (testes).
    ///
    /// **Onde:** os testes com binário falso que dorme.
    pub const fn com_timeout(mut self, teto: Duration) -> Self {
        self.versao_timeout = teto;
        self
    }
}

/// Os hooks históricos do `overdev` (`overdev guard|check` e `guard|check|hooks` no
/// `schematize-overdev`), para o host copiar em vez de redigitar.
pub const HOOKS_OVERDEV: &[&[&str]] = &[&["overdev", "guard|check"], &["guard|check|hooks"]];

/// Desfecho de uma tentativa de garantir o gestor.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resultado {
    /// Os dois já existiam: nada feito, nenhuma rede.
    pub ja_estava: bool,
    /// Binários instalados nesta chamada.
    pub instalados: Vec<&'static str>,
    /// Binários que falharam, com a causa (uma falha não impede os demais).
    pub falhas: Vec<(&'static str, String)>,
    /// Avisos que não são falha de binário (lock que não saiu, stderr fechado, ...).
    pub avisos: Vec<String>,
    /// Outro processo já cuida da instalação (lock recente).
    pub outro_cuida: bool,
}

/// **O quê:** `true` se `args` casa algum padrão de hook de `cfg` (prefixo, com alternativas `|`).
///
/// **Onde:** [`deve_rodar`].
pub fn eh_hook(cfg: &Config<'_>, args: &[String]) -> bool {
    cfg.hooks.iter().any(|p| {
        !p.is_empty()
            && args.len() >= p.len()
            && p.iter().zip(args).all(|(alt, arg)| alt.split('|').any(|a| a == arg))
    })
}

/// **O quê:** decide se o parasita roda, sobre o ambiente (`env`) e os `args` (sem o argv[0]).
/// Não roda em `--version/-V/--help/-h`, `help`, `desktop --install/--remover` (só esses; `desktop`
/// sozinho roda), hooks do host, sob o market (`SCHEMATIZE_DELEGADO_AO_MARKET=1`), nem com
/// `SCHEMATIZE_SEM_MARKET=1` ou `CI` definida.
///
/// **Onde:** [`garantir_com`]. Pura: o ambiente entra por parâmetro.
pub fn deve_rodar(cfg: &Config<'_>, env: &dyn Fn(&str) -> Option<String>, args: &[String]) -> bool {
    if env("SCHEMATIZE_SEM_MARKET").as_deref() == Some("1")
        || env("SCHEMATIZE_DELEGADO_AO_MARKET").as_deref() == Some("1")
        || env("CI").is_some()
    {
        return false;
    }
    let tem = |f: &str| args.iter().any(|a| a == f);
    if tem("--version") || tem("-V") || tem("--help") || tem("-h") {
        return false;
    }
    let primeiro = args.first().map(String::as_str);
    if primeiro == Some("help") {
        return false;
    }
    if primeiro == Some("desktop") && (tem("--install") || tem("--remover")) {
        return false;
    }
    !eh_hook(cfg, args)
}

/// **O quê:** o miolo testável: garante os binários em `pasta` com `lock`, para
/// `plataforma = (os, arch)`, usando o downloader injetado. Continua após falha de um binário.
///
/// **Onde:** [`garantir_com`] e os testes. Efeito: pode criar `pasta`, escrever binários e
/// criar/remover `lock`.
pub fn garantir_em(
    cfg: &Config<'_>,
    pasta: &Path,
    lock: &Path,
    (os, arch): (&str, &str),
    baixar: Baixador<'_>,
) -> Resultado {
    let mut res = Resultado::default();
    let falta = o_que_falta_para(pasta, os);
    if falta.is_empty() {
        res.ja_estava = true;
        return res;
    }
    match tomar_lock(lock) {
        Ok(Lock::Meu) => {}
        Ok(Lock::Ocupado) => {
            res.outro_cuida = true;
            return res;
        }
        Err(causa) => {
            res.falhas.push((falta[0], causa));
            return res;
        }
    }
    if let Err(e) = std::fs::create_dir_all(pasta) {
        res.falhas.push((falta[0], format!("criar {}: {e}", pasta.display())));
    } else {
        for nome in falta {
            match instalar_nome(cfg, nome, pasta, (os, arch), baixar) {
                Ok(()) => res.instalados.push(nome),
                Err(causa) => res.falhas.push((nome, causa)),
            }
        }
    }
    match std::fs::remove_file(lock) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => res.avisos.push(format!("não removi o lock {}: {e}", lock.display())),
    }
    res
}

/// **O quê:** resolve URL e destino de UM nome e instala.
///
/// **Onde:** laço de [`garantir_em`].
fn instalar_nome(
    cfg: &Config<'_>,
    nome: &str,
    pasta: &Path,
    (os, arch): (&str, &str),
    baixar: Baixador<'_>,
) -> Result<(), String> {
    let url = url_do_asset(nome, os, arch)
        .ok_or_else(|| format!("sem binário publicado para {os}-{arch}"))?;
    let dst = pasta.join(nome_do_arquivo(nome, os));
    instalar_um(&url, &dst, os, cfg.versao_timeout, baixar)
}

/// **O quê:** as linhas do stderr para um [`Resultado`]: uma de sucesso, UMA POR FALHA e uma
/// por aviso. Vazio = silêncio (já estava / outro cuida).
///
/// **Onde:** [`garantir_com`]; pura para ser testada.
pub fn linhas(cfg: &Config<'_>, res: &Resultado, pasta: &Path) -> Vec<String> {
    let app = cfg.app;
    let mut v = Vec::new();
    if !res.instalados.is_empty() {
        v.push(format!(
            "{app}: instalei o gestor de atualizações ({}) em {}",
            res.instalados.join(", "),
            pasta.display()
        ));
    }
    for (nome, causa) in &res.falhas {
        v.push(format!(
            "{app}: não consegui instalar o {nome} ({causa}) — o app segue normal; rode: {INSTALAR}"
        ));
    }
    for a in &res.avisos {
        v.push(format!("{app}: aviso: {a}"));
    }
    v
}

/// **O quê:** escreve as `linhas` em `saida`, uma por linha.
///
/// **Onde:** [`garantir_com`] (com o stderr); os testes passam um `Vec<u8>`. Devolve o erro de
/// E/S (stderr fechado) em vez de dar `panic` como o `eprintln!`.
pub fn emitir(linhas: &[String], saida: &mut dyn Write) -> std::io::Result<()> {
    for l in linhas {
        writeln!(saida, "{l}")?;
    }
    Ok(())
}

/// **O quê:** o pipeline completo sobre ambiente e downloader injetados: decide, resolve as
/// pastas a partir de `home`, instala e escreve as linhas no stderr. `None` = não rodou.
///
/// **Onde:** [`garantir`] (real) e os testes (opt-out e fluxo). `home = None` vira uma falha
/// relatada, nunca `panic`.
pub fn garantir_com(
    cfg: &Config<'_>,
    env: &dyn Fn(&str) -> Option<String>,
    args: &[String],
    home: Option<PathBuf>,
    plataforma: (&str, &str),
    baixar: Baixador<'_>,
) -> Option<Resultado> {
    if !deve_rodar(cfg, env, args) {
        return None;
    }
    let (pasta, mut res) = match home {
        Some(h) => {
            let pasta = h.join(".cargo").join("bin");
            let lock = h.join(".schematize").join("run").join("market-bootstrap.lock");
            let r = garantir_em(cfg, &pasta, &lock, plataforma, baixar);
            (pasta, r)
        }
        None => {
            let falha = (NOMES[0], "HOME/USERPROFILE indefinido".to_string());
            (PathBuf::from("?"), Resultado { falhas: vec![falha], ..Resultado::default() })
        }
    };
    if let Err(e) = emitir(&linhas(cfg, &res, &pasta), &mut std::io::stderr()) {
        res.avisos.push(format!("stderr indisponível: {e}"));
    }
    Some(res)
}

/// **O quê:** garante o market na pasta canônica do usuário (`HOME`, senão `USERPROFILE`).
/// Chame no início do `main`, depois do parse de argumentos. Em `cfg(test)` desta lib não faz
/// nada. Nunca dá `panic` para o host: um `panic` interno vira linha no stderr.
///
/// **Onde:** `main` de cada binário. Síncrono e limitado (30 s por download + 10 s de
/// `--version` por binário), só quando falta.
pub fn garantir(cfg: &Config<'_>) {
    #[cfg(not(test))]
    {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let env = |k: &str| std::env::var(k).ok();
            let home = crate::home::home_do_ambiente();
            garantir_com(cfg, &env, &args, home, plataforma_atual(), &baixar_com_curl)
        }));
        if r.is_err() {
            // Sem canal melhor: o `panic` já imprimiu a causa; aqui só dizemos que o app segue.
            // Último recurso: se nem o stderr aceita escrita, não há canal para relatar.
            let _ = writeln!(std::io::stderr(), "{}: o parasita falhou — o app segue", cfg.app);
        }
    }
    #[cfg(test)]
    let _ = cfg;
}
