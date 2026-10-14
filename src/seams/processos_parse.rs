//! Parsers PUROS (texto -> dado) de `processos`: nenhum toca o sistema.
//!
//! **O quê:** interpreta `/proc` (Linux), `ps`, `lsof`, `procstat`, `sysctl`, `vm_stat` e
//! `tasklist`. Texto hostil (lixo, número gigante/negativo, vazio) vira vazio/`None`, nunca pânico.
//!
//! **Onde:** `processos::ProcessosCom`. Rodam em QUALQUER SO nos testes, com texto de fixture.

use std::path::PathBuf;

/// **O quê:** basename de um caminho, aceitando `/` e `\`.
///
/// **Onde:** [`cmdline_casa`].
pub(super) fn nome_base(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or(s)
}

/// **O quê:** quebra um `/proc/<pid>/cmdline` (separado por NUL) em args, descartando vazios e
/// convertendo UTF-8 inválido com perda (movido de `agents::count_claude_processes`).
///
/// **Onde:** `ProcessosCom` no ramo Linux.
pub(super) fn args_de_cmdline(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// **O quê:** decide se um cmdline é o processo `nome`: argv0 com basename `nome`, OU runtime
/// (`node`/`nodejs`/`bun`/`deno`) cujo script seguinte é `nome` (`.../nome`, `nome/cli.js`,
/// `nome-code`, `/nome/cli`). Movido de `agents::cmdline_e_claude`, generalizado de `claude`.
///
/// **Onde:** `contar` e `em_dir` de `ProcessosCom`. `nome` vazio nunca casa.
pub(super) fn cmdline_casa(args: &[String], nome: &str) -> bool {
    let Some(argv0) = args.first() else { return false };
    if nome.is_empty() {
        return false;
    }
    let base0 = nome_base(argv0);
    if base0 == nome {
        return true;
    }
    if !matches!(base0, "node" | "nodejs" | "bun" | "deno") {
        return false;
    }
    let cli = format!("/{nome}/cli.js");
    let code = format!("{nome}-code");
    let dir_cli = format!("/{nome}/cli");
    args.iter().skip(1).any(|a| {
        nome_base(a) == nome || a.ends_with(&cli) || a.contains(&code) || a.contains(&dir_cli)
    })
}

/// **O quê:** parseia `ps -axo pid=,comm=,args=` em `(pid, args)`. Linha cuja 1a coluna não é
/// `u32` (cabeçalho, lixo) é pulada. Sem coluna `args`, usa o `comm`. Limitação: `comm` com
/// espaço (ex. "Web Content") desalinha o argv0 (falso negativo, nunca falso positivo grave).
///
/// **Onde:** `contar`/`em_dir` em FreeBSD, macOS e demais unix.
pub(super) fn parse_ps(texto: &str) -> Vec<(u32, Vec<String>)> {
    texto
        .lines()
        .filter_map(|l| {
            let mut t = l.split_whitespace();
            let pid = t.next()?.parse::<u32>().ok()?;
            let resto: Vec<&str> = t.collect();
            let args = if resto.len() > 1 { &resto[1..] } else { &resto[..] };
            if args.is_empty() {
                return None;
            }
            Some((pid, args.iter().map(|s| s.to_string()).collect()))
        })
        .collect()
}

/// **O quê:** `true` se a saída de `ps -p <pid> -o pid=` lista exatamente `pid`.
///
/// **Onde:** `vivo` em FreeBSD, macOS e demais unix.
pub(super) fn pid_em_ps(texto: &str, pid: u32) -> bool {
    texto.lines().any(|l| l.trim().parse::<u32>() == Ok(pid))
}

/// **O quê:** cwds de `lsof -a -d cwd -Fn -p ...` (linhas `n<caminho>`).
///
/// **Onde:** `em_dir` em macOS e demais unix.
pub(super) fn parse_lsof_cwd(texto: &str) -> Vec<PathBuf> {
    texto
        .lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// **O quê:** cwds de `procstat -f <pids>`: linhas cuja 3a coluna (FD) é `cwd`; o caminho é o
/// resto após 9 colunas (PID COMM FD T V FLAGS REF OFFSET PRO), podendo ter espaços.
///
/// **Onde:** `em_dir` em FreeBSD.
pub(super) fn parse_procstat_cwd(texto: &str) -> Vec<PathBuf> {
    texto
        .lines()
        .filter(|l| l.split_whitespace().nth(2) == Some("cwd"))
        .filter_map(|l| resto_apos(l, 9))
        .filter(|p| p.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

/// **O quê:** o texto que sobra depois de pular `n` colunas separadas por espaço.
///
/// **Onde:** [`parse_procstat_cwd`].
fn resto_apos(linha: &str, n: usize) -> Option<&str> {
    let mut s = linha.trim_start();
    for _ in 0..n {
        let fim = s.find(char::is_whitespace)?;
        s = s[fim..].trim_start();
    }
    let s = s.trim_end();
    (!s.is_empty()).then_some(s)
}

/// **O quê:** MB de `u128` com saturação em `u64`.
///
/// **Onde:** os parsers de memória BSD/macOS.
fn mb_de_bytes(bytes: u128) -> u64 {
    u64::try_from(bytes / (1024 * 1024)).unwrap_or(u64::MAX)
}

/// **O quê:** MB disponíveis de `/proc/meminfo` (`MemAvailable`, kB/1024); `None` se ausente ou
/// ilegível (movido de `agents::mem_available_mb`, que fingia `u64::MAX/2`).
///
/// **Onde:** `memoria_disponivel_mb` no Linux.
pub(super) fn parse_meminfo_mb(texto: &str) -> Option<u64> {
    texto.lines().find_map(|l| {
        let rest = l.strip_prefix("MemAvailable:")?;
        rest.split_whitespace().next()?.parse::<u64>().ok().map(|kb| kb / 1024)
    })
}

/// **O quê:** MB de `sysctl -n hw.pagesize vm.stats.vm.v_free_count vm.stats.vm.v_inactive_count`
/// (3 linhas): `pagesize * (free + inactive)`. Menos de 3 números válidos ou pagesize 0 -> `None`.
///
/// **Onde:** `memoria_disponivel_mb` no FreeBSD.
pub(super) fn parse_freebsd_mb(texto: &str) -> Option<u64> {
    let n: Vec<u128> = texto.lines().map_while(|l| l.trim().parse::<u128>().ok()).collect();
    let [pagina, livres, inativas] = n.get(..3)? else { return None };
    if *pagina == 0 {
        return None;
    }
    Some(mb_de_bytes(pagina.saturating_mul(livres.saturating_add(*inativas))))
}

/// **O quê:** MB de `vm_stat` (macOS): `page size of N bytes` * (free + inactive + speculative).
/// `free` e `inactive` são obrigatórios; `speculative` ausente conta 0.
///
/// **Onde:** `memoria_disponivel_mb` no macOS.
pub(super) fn parse_vm_stat_mb(texto: &str) -> Option<u64> {
    let pagina = texto.lines().find_map(|l| {
        l.split("page size of ").nth(1)?.split_whitespace().next()?.parse::<u128>().ok()
    })?;
    if pagina == 0 {
        return None;
    }
    let paginas = |chave: &str| -> Option<u128> {
        texto.lines().find_map(|l| {
            let v = l.strip_prefix(chave)?.trim().trim_end_matches('.');
            v.parse::<u128>().ok()
        })
    };
    let total = paginas("Pages free:")?
        .saturating_add(paginas("Pages inactive:")?)
        .saturating_add(paginas("Pages speculative:").unwrap_or(0));
    Some(mb_de_bytes(pagina.saturating_mul(total)))
}

/// **O quê:** load de 1 min do 1o token de `/proc/loadavg` ou `sysctl -n vm.loadavg`
/// (`{ 0.52 0.58 0.59 }`). NaN, infinito e negativo -> `None`.
///
/// **Onde:** `carga1` (todos os SOs unix).
pub(super) fn parse_carga(texto: &str) -> Option<f64> {
    let limpo = texto.replace(['{', '}'], " ");
    let v = limpo.split_whitespace().next()?.parse::<f64>().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v)
}

/// **O quê:** `(imagem, pid)` de `tasklist /NH /FO CSV` (`"nome.exe","1234",...`); linhas que
/// não são CSV de tasklist (ex. `INFO: No tasks...`) são puladas.
///
/// **Onde:** `vivo`/`contar` no Windows.
pub(super) fn parse_tasklist(texto: &str) -> Vec<(String, u32)> {
    texto
        .lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix('"')?;
            let mut c = l.split("\",\"");
            let nome = c.next()?.to_string();
            let pid = c.next()?.trim_matches('"').parse::<u32>().ok()?;
            Some((nome, pid))
        })
        .collect()
}
