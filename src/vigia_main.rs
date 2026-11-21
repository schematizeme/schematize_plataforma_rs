//! `schematize-vigia`: CLI mínima (std-only, sem dependências) sobre `persistencia::vigia`.
//!
//! **O quê:** lança um comando, relança com `--resume` quando ele morre e abre o circuito
//! (`parked`) se morrer demais. Também imprime a unit systemd (sem instalar nada), mostra o
//! estado do journal do vigia e limpa o marcador `parked`.
//!
//! **Onde:** `ExecStart` da unit gerada por `--unit-systemd` e uso manual do ops.
//!
//! Exit codes: 0 concluiu (ou ajuda/versão/status/unit/limpeza ok), 1 erro de execução/E-S,
//! 2 uso incorreto, 3 parked (circuito aberto, agora ou de antes; também no `--status`).

use schematize_plataforma::persistencia::{
    rodar, unit_systemd, Config, Desfecho, Erro, Journal, VigiaCfg,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// O que o usuário pediu.
#[derive(Debug, PartialEq, Eq)]
enum Acao {
    Ajuda,
    Versao,
    Rodar,
    Unit,
    Status,
    Limpar,
}

/// Argumentos já interpretados.
#[derive(Debug)]
struct Args {
    acao: Acao,
    estado: Option<PathBuf>,
    max_mortes: Option<u32>,
    janela_s: Option<u64>,
    backoff_ms: Option<u64>,
    sim: bool,
    cmd: Vec<String>,
}

/// **O quê:** escolhe o texto no idioma do ambiente (`SCHEMATIZE_LANG`, `LC_ALL`, `LANG`;
/// começa com `pt` = português, senão inglês). **Onde:** toda mensagem ao usuário.
fn t<'a>(pt: &'a str, en: &'a str) -> &'a str {
    let lang = ["SCHEMATIZE_LANG", "LC_ALL", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    if lang.to_lowercase().starts_with("pt") {
        pt
    } else {
        en
    }
}

/// **O quê:** lê o valor numérico que acompanha a flag `nome`. **Onde:** `interpretar`.
fn numero<T: std::str::FromStr>(nome: &str, v: Option<&String>, minimo: u64) -> Result<T, String> {
    let bruto = v.ok_or_else(|| format!("{nome}: {}", t("falta o valor", "missing value")))?;
    let n: u64 = bruto.parse().map_err(|_| {
        format!(
            "{nome}: {} '{bruto}' {}",
            t("o valor", "value"),
            t("não é um inteiro positivo", "is not a positive integer")
        )
    })?;
    if n < minimo {
        return Err(format!("{nome}: {} {minimo}", t("o mínimo é", "minimum is")));
    }
    n.to_string().parse().map_err(|_| format!("{nome}: {}", t("valor grande demais", "too big")))
}

/// **O quê:** interpreta os argumentos (sem o nome do programa); tudo após `--` é o comando.
///
/// **Onde:** `main`. Erro = mensagem de uso (exit 2), sem culpa e com o caminho certo.
fn interpretar(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        acao: Acao::Rodar,
        estado: None,
        max_mortes: None,
        janela_s: None,
        backoff_ms: None,
        sim: false,
        cmd: Vec::new(),
    };
    let (mut unit, mut status, mut limpar) = (false, false, false);
    let mut i = 0;
    while i < argv.len() {
        let flag = argv[i].as_str();
        let valor = argv.get(i + 1);
        match flag {
            "--" => {
                a.cmd = argv[i + 1..].to_vec();
                break;
            }
            "--help" | "-h" => return Ok(Args { acao: Acao::Ajuda, ..a }),
            "--version" | "-V" => return Ok(Args { acao: Acao::Versao, ..a }),
            "--unit-systemd" => unit = true,
            "--status" => status = true,
            "--limpar-parked" => limpar = true,
            "--sim" => a.sim = true,
            "--estado" => {
                let v = valor.ok_or_else(|| {
                    t("--estado: falta o diretório", "--estado: missing directory")
                })?;
                a.estado = Some(PathBuf::from(v));
                i += 1;
            }
            "--max-mortes" => {
                a.max_mortes = Some(numero(flag, valor, 1)?);
                i += 1;
            }
            "--janela-s" => {
                a.janela_s = Some(numero(flag, valor, 1)?);
                i += 1;
            }
            "--backoff-ms" => {
                a.backoff_ms = Some(numero(flag, valor, 0)?);
                i += 1;
            }
            outro => {
                return Err(format!(
                    "{} '{outro}'. {}",
                    t("opção desconhecida", "unknown option"),
                    t(
                        "Veja --help; o comando a vigiar vai depois de '--'.",
                        "See --help; the command to watch goes after '--'."
                    )
                ))
            }
        }
        i += 1;
    }
    if [unit, status, limpar].iter().filter(|b| **b).count() > 1 {
        return Err(t(
            "use só uma entre --unit-systemd, --status e --limpar-parked",
            "use only one of --unit-systemd, --status and --limpar-parked",
        )
        .into());
    }
    a.acao = match (unit, status, limpar) {
        (true, _, _) => Acao::Unit,
        (_, true, _) => Acao::Status,
        (_, _, true) => Acao::Limpar,
        _ => Acao::Rodar,
    };
    Ok(a)
}

/// **O quê:** exige `--estado`. **Onde:** todas as ações exceto ajuda/versão.
fn exige_estado(a: &Args) -> Result<&Path, String> {
    a.estado.as_deref().ok_or_else(|| {
        t(
            "falta --estado <dir>: diga onde o vigia guarda o journal e o marcador parked",
            "missing --estado <dir>: say where the watchdog keeps its journal and parked marker",
        )
        .into()
    })
}

/// **O quê:** monta o `VigiaCfg` a partir dos argumentos. **Onde:** rodar e unit.
fn montar_cfg(a: &Args, estado: &Path) -> Result<VigiaCfg, String> {
    let (prog, args) = a.cmd.split_first().ok_or_else(|| {
        t(
            "falta o comando: schematize-vigia --estado <dir> -- <cmd> [args...]",
            "missing command: schematize-vigia --estado <dir> -- <cmd> [args...]",
        )
        .to_string()
    })?;
    let mut c = VigiaCfg::novo(prog, estado);
    c.args = args.to_vec();
    if let Ok(exe) = std::env::current_exe() {
        c.exe_vigia = exe;
    }
    if let Some(m) = a.max_mortes {
        c.max_mortes = m;
    }
    if let Some(s) = a.janela_s {
        c.janela = Duration::from_secs(s);
    }
    if let Some(b) = a.backoff_ms {
        c.backoff_base = Duration::from_millis(b);
        c.backoff_teto = c.backoff_teto.max(c.backoff_base);
    }
    Ok(c)
}

/// Resumo do histórico do vigia.
#[derive(Debug, Default, PartialEq, Eq)]
struct Resumo {
    lancamentos: u32,
    mortes: u32,
    ultima_causa: Option<String>,
    concluiu: bool,
}

/// **O quê:** conta lançamentos/mortes e acha a última causa nas linhas do journal.
///
/// **Onde:** `status`. Puro (testável sem disco).
fn resumir(linhas: &[String]) -> Resumo {
    let mut r = Resumo::default();
    for l in linhas {
        if l.starts_with("lancamento ") {
            r.lancamentos += 1;
        } else if l.starts_with("morte ") {
            r.mortes += 1;
            r.ultima_causa =
                l.split_whitespace().find_map(|p| p.strip_prefix("causa=")).map(str::to_string);
        } else if l.starts_with("fim_ok") {
            r.concluiu = true;
        }
    }
    r
}

/// **O quê:** `--status`: lê o journal do vigia e o marcador `parked`. Exit 3 se parked.
///
/// **Onde:** `main`. Se o vigia está rodando (lock ocupado) não mexe no journal: mostra o
/// que dá e avisa, sem erro.
fn status(estado: &Path) -> ExitCode {
    println!("{}: {}", t("estado", "state"), estado.display());
    let parked = std::fs::read_to_string(estado.join("parked")).ok();
    let dir_j = estado.join("vigia");
    if !dir_j.is_dir() {
        println!(
            "{}",
            t(
                "sem histórico: o vigia ainda não rodou aqui",
                "no history: the watchdog has not run here yet"
            )
        );
    } else {
        match Journal::abrir(&dir_j, Config::default()).and_then(|mut j| j.recuperar()) {
            Ok((regs, _)) => {
                let linhas: Vec<String> =
                    regs.iter().map(|r| String::from_utf8_lossy(&r.payload).into_owned()).collect();
                let r = resumir(&linhas);
                println!("{}: {}", t("lançamentos", "launches"), r.lancamentos);
                println!("{}: {}", t("mortes", "deaths"), r.mortes);
                println!(
                    "{}: {}",
                    t("última causa", "last cause"),
                    r.ultima_causa.as_deref().unwrap_or("-")
                );
                if r.concluiu {
                    println!(
                        "{}",
                        t(
                            "último ciclo: concluído com sucesso",
                            "last cycle: finished successfully"
                        )
                    );
                }
            }
            Err(Erro::LockOcupado { pid, .. }) => println!(
                "{} (pid {}): {}",
                t("vigia em execução", "watchdog running"),
                pid.map_or("?".to_string(), |p| p.to_string()),
                t(
                    "journal em uso, histórico indisponível agora",
                    "journal in use, history unavailable now"
                )
            ),
            Err(e) => {
                eprintln!("schematize-vigia: {e}");
                return ExitCode::from(1);
            }
        }
    }
    match parked {
        Some(p) => {
            println!("parked: {}", t("sim", "yes"));
            println!("  {}", p.trim());
            println!(
                "{}",
                t(
                    "para retomar: schematize-vigia --estado <dir> --limpar-parked --sim",
                    "to resume: schematize-vigia --estado <dir> --limpar-parked --sim"
                )
            );
            ExitCode::from(3)
        }
        None => {
            println!("parked: {}", t("não", "no"));
            ExitCode::SUCCESS
        }
    }
}

/// **O quê:** `--limpar-parked --sim`: remove o marcador (confirmação explícita).
///
/// **Onde:** `main`. Sem marcador = nada a fazer, não é erro.
fn limpar(estado: &Path, sim: bool) -> ExitCode {
    if !sim {
        eprintln!(
            "{}",
            t(
                "isto reabre o circuito e o vigia volta a relançar o comando. Confirme com --sim: schematize-vigia --estado <dir> --limpar-parked --sim",
                "this closes the breaker and the watchdog will relaunch the command again. Confirm with --sim: schematize-vigia --estado <dir> --limpar-parked --sim"
            )
        );
        return ExitCode::from(2);
    }
    match std::fs::remove_file(estado.join("parked")) {
        Ok(()) => println!(
            "{}",
            t(
                "parked removido; o vigia pode rodar de novo",
                "parked removed; the watchdog can run again"
            )
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "{}",
                t("não havia parked; nada a fazer", "there was no parked marker; nothing to do")
            )
        }
        Err(e) => {
            eprintln!(
                "schematize-vigia: {} ({e})",
                t("não consegui remover o parked", "could not remove parked")
            );
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}

/// **O quê:** `rodar` e traduz o desfecho em mensagem + exit code. **Onde:** `main`.
fn executar(cfg: &VigiaCfg) -> ExitCode {
    match rodar(cfg) {
        Ok(Desfecho::Concluido { lancamentos }) => {
            eprintln!(
                "schematize-vigia: {} ({lancamentos})",
                t("concluído, lançamentos", "finished, launches")
            );
            ExitCode::SUCCESS
        }
        Ok(Desfecho::Parked { mortes }) => {
            eprintln!(
                "schematize-vigia: {} {mortes} {}",
                t("circuito aberto após", "breaker open after"),
                t(
                    "mortes. Veja a causa com --status; depois de corrigir, --limpar-parked --sim.",
                    "deaths. See the cause with --status; once fixed, --limpar-parked --sim."
                )
            );
            ExitCode::from(3)
        }
        Ok(Desfecho::JaParked) => {
            eprintln!(
                "schematize-vigia: {}",
                t(
                    "já estava parked de uma rodada anterior; nada foi lançado. Veja --status e, se já corrigiu, --limpar-parked --sim.",
                    "already parked from an earlier run; nothing was launched. See --status and, once fixed, --limpar-parked --sim."
                )
            );
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("schematize-vigia: {e}");
            ExitCode::from(1)
        }
    }
}

/// **O quê:** texto da ajuda (pt/en). **Onde:** `--help` e erros de uso.
fn ajuda() -> String {
    format!(
        "schematize-vigia {v}\n\n{u}\n  schematize-vigia --estado <dir> [--max-mortes N] [--janela-s S] [--backoff-ms B] -- <cmd> [args...]\n  schematize-vigia --estado <dir> --unit-systemd -- <cmd> [args...]\n  schematize-vigia --estado <dir> --status\n  schematize-vigia --estado <dir> --limpar-parked --sim\n  schematize-vigia --help | --version\n\n{d}\n\nExit codes: 0 {c0} | 1 {c1} | 2 {c2} | 3 parked\n",
        v = env!("CARGO_PKG_VERSION"),
        u = t("Uso:", "Usage:"),
        d = t(
            "Relança <cmd> com --resume quando morre (backoff + jitter); abre o circuito (parked) após N mortes na janela (default 5 em 600 s). --unit-systemd só imprime o texto; nada é instalado.",
            "Relaunches <cmd> with --resume when it dies (backoff + jitter); opens the breaker (parked) after N deaths in the window (default 5 in 600 s). --unit-systemd only prints text; nothing is installed."
        ),
        c0 = t("concluiu", "done"),
        c1 = t("erro", "error"),
        c2 = t("uso incorreto", "bad usage"),
    )
}

/// **O quê:** ponto de entrada: interpreta e despacha. **Onde:** processo `schematize-vigia`.
fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match interpretar(&argv) {
        Ok(a) => a,
        Err(m) => {
            eprintln!("schematize-vigia: {m}\n\n{}", ajuda());
            return ExitCode::from(2);
        }
    };
    match a.acao {
        Acao::Ajuda => {
            print!("{}", ajuda());
            return ExitCode::SUCCESS;
        }
        Acao::Versao => {
            println!("schematize-vigia {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let estado = match exige_estado(&a) {
        Ok(e) => e.to_path_buf(),
        Err(m) => {
            eprintln!("schematize-vigia: {m}");
            return ExitCode::from(2);
        }
    };
    match a.acao {
        Acao::Status => status(&estado),
        Acao::Limpar => limpar(&estado, a.sim),
        Acao::Unit | Acao::Rodar => match montar_cfg(&a, &estado) {
            Ok(cfg) if a.acao == Acao::Unit => {
                print!("{}", unit_systemd(&cfg));
                ExitCode::SUCCESS
            }
            Ok(cfg) => executar(&cfg),
            Err(m) => {
                eprintln!("schematize-vigia: {m}");
                ExitCode::from(2)
            }
        },
        Acao::Ajuda | Acao::Versao => ExitCode::SUCCESS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converte literais em `Vec<String>`. **Onde:** testes.
    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// Parse feliz: opções antes de `--`, comando e args depois (inclusive flags do filho).
    #[test]
    fn parse_completo() {
        let a =
            interpretar(&v(&["--estado", "/e", "--max-mortes", "3", "--", "x", "--max-mortes"]))
                .expect("ok");
        assert_eq!((a.acao, a.max_mortes), (Acao::Rodar, Some(3)));
        assert_eq!(a.cmd, ["x", "--max-mortes"]);
    }

    /// Entradas hostis viram erro de uso, nunca pânico.
    #[test]
    fn parse_hostil() {
        for bad in [
            &["--max-mortes"][..],
            &["--max-mortes", "0"],
            &["--janela-s", "abc"],
            &["--zzz"],
            &["--status", "--unit-systemd"],
            &["--estado"],
            &["--backoff-ms", "-1"],
        ] {
            assert!(interpretar(&v(bad)).is_err(), "{bad:?}");
        }
    }

    /// Resumo conta mortes e pega a última causa.
    #[test]
    fn resumo() {
        let l = v(&[
            "lancamento n=0",
            "morte sinal=9 codigo=- causa=a vivo_ms=1",
            "lancamento n=1",
            "morte sinal=- codigo=2 causa=exit_2 vivo_ms=1",
            "parked morte x",
        ]);
        let r = resumir(&l);
        assert_eq!((r.lancamentos, r.mortes, r.ultima_causa.as_deref()), (2, 2, Some("exit_2")));
    }
}
