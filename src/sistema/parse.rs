//! Parsers puros (texto -> valor) dos arquivos de /proc, /sys e /etc. Nunca panic: entrada
//! truncada, não numérica ou hostil vira [`ErroSistema::Invalido`] ou item ignorado.

use crate::sistema::erro::ErroSistema;
use crate::sistema::tipos::*;
use std::collections::HashMap;

/// Tamanho máximo de string textual guardada no perfil.
const MAX_TEXTO: usize = 256;

/// O quê: remove controles (incl. NUL), apara e trunca texto vindo do host.
/// Onde: todo valor textual que entra no perfil.
pub fn sanear(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_TEXTO).collect::<String>().trim().to_string()
}

/// O quê: `u64` decimal estrito (sem sinal, sem lixo). Onde: cgroup files.
pub fn parse_u64(txt: &str, campo: &str) -> Result<u64, ErroSistema> {
    let t = txt.trim();
    t.parse::<u64>().map_err(|_| {
        ErroSistema::invalido(campo, &format!("não é inteiro sem sinal: {:?}", sanear(t)))
    })
}

/// O quê: `i64` decimal estrito. Onde: sysctl e oom_score.
pub fn parse_i64(txt: &str, campo: &str) -> Result<i64, ErroSistema> {
    let t = txt.trim();
    t.parse::<i64>()
        .map_err(|_| ErroSistema::invalido(campo, &format!("não é inteiro: {:?}", sanear(t))))
}

/// O quê: `max` ou número. Onde: memory.max, pids.max etc.
pub fn parse_limite(txt: &str, campo: &str) -> Result<Limite, ErroSistema> {
    if txt.trim() == "max" {
        Ok(Limite::Max)
    } else {
        parse_u64(txt, campo).map(Limite::Valor)
    }
}

/// O quê: mapa `chave valor` por linha (memory.events). Linhas inválidas são ignoradas.
/// Onde: eventos de cgroup.
pub fn parse_chave_valor_u64(txt: &str) -> HashMap<String, u64> {
    txt.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let k = it.next()?;
            let v = it.next()?.parse::<u64>().ok()?;
            Some((sanear(k), v))
        })
        .collect()
}

/// O quê: parse de memory.events. Onde: coletor de cgroup.
pub fn parse_eventos(txt: &str) -> Result<EventosMem, ErroSistema> {
    let m = parse_chave_valor_u64(txt);
    if m.is_empty() {
        return Err(ErroSistema::invalido("memory.events", "sem contadores"));
    }
    Ok(EventosMem {
        low: m.get("low").copied(),
        high: m.get("high").copied(),
        max: m.get("max").copied(),
        oom: m.get("oom").copied(),
        oom_kill: m.get("oom_kill").copied(),
    })
}

/// O quê: parse de /proc/meminfo (kB). Exige ao menos MemTotal. Onde: coletor de memória.
pub fn parse_meminfo(txt: &str) -> Result<Meminfo, ErroSistema> {
    let mut m: HashMap<&str, u64> = HashMap::new();
    for l in txt.lines() {
        if let Some((k, v)) = l.split_once(':') {
            if let Some(n) = v.split_whitespace().next().and_then(|x| x.parse::<u64>().ok()) {
                m.insert(k.trim(), n);
            }
        }
    }
    let total = m.get("MemTotal").copied();
    if total.is_none() {
        return Err(ErroSistema::invalido("meminfo", "MemTotal ausente ou ilegível"));
    }
    Ok(Meminfo {
        total_kib: total,
        disponivel_kib: m.get("MemAvailable").copied(),
        swap_total_kib: m.get("SwapTotal").copied(),
        swap_livre_kib: m.get("SwapFree").copied(),
    })
}

/// O quê: parse de /proc/swaps (pula cabeçalho e linhas truncadas). Onde: prioridade do zram.
pub fn parse_swaps(txt: &str) -> Vec<SwapDev> {
    txt.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() < 5 {
                return None;
            }
            Some(SwapDev {
                arquivo: sanear(c[0]),
                tamanho_kib: c[2].parse().ok()?,
                usado_kib: c[3].parse().ok()?,
                prioridade: c[4].parse().ok()?,
            })
        })
        .collect()
}

/// O quê: algoritmo ativo de `comp_algorithm` (`lzo [zstd] lz4` -> `zstd`). Onde: zram.
pub fn parse_algoritmo_zram(txt: &str) -> Option<String> {
    let ini = txt.find('[')?;
    let fim = txt[ini..].find(']')? + ini;
    let a = sanear(&txt[ini + 1..fim]);
    (!a.is_empty()).then_some(a)
}

/// O quê: parse de uma linha PSI `some avg10=.. avg60=.. avg300=.. total=..`. Onde: `parse_psi`.
fn parse_linha_psi(l: &str) -> Option<PsiLinha> {
    let mut avg = [None::<f64>; 3];
    let mut total = None;
    for p in l.split_whitespace().skip(1) {
        let (k, v) = p.split_once('=')?;
        match k {
            "avg10" => avg[0] = v.parse().ok().filter(|x: &f64| x.is_finite()),
            "avg60" => avg[1] = v.parse().ok().filter(|x: &f64| x.is_finite()),
            "avg300" => avg[2] = v.parse().ok().filter(|x: &f64| x.is_finite()),
            "total" => total = v.parse().ok(),
            _ => {}
        }
    }
    Some(PsiLinha { avg10: avg[0]?, avg60: avg[1]?, avg300: avg[2]?, total_us: total.unwrap_or(0) })
}

/// O quê: parse de /proc/pressure/*. Exige ao menos `some` válido. Onde: coletor PSI.
pub fn parse_psi(txt: &str) -> Result<PsiRecurso, ErroSistema> {
    let mut r = PsiRecurso::default();
    for l in txt.lines() {
        match l.split_whitespace().next() {
            Some("some") => r.some = parse_linha_psi(l),
            Some("full") => r.full = parse_linha_psi(l),
            _ => {}
        }
    }
    if r.some.is_none() && r.full.is_none() {
        return Err(ErroSistema::invalido("psi", "nenhuma linha some/full válida"));
    }
    Ok(r)
}

/// O quê: caminho do cgroup v2 (`0::/x`) sem `(deleted)`; rejeita `..` e vazio.
/// Onde: coletor de cgroup (/proc/self/cgroup).
pub fn parse_cgroup_proprio(txt: &str) -> Result<String, ErroSistema> {
    let linha = txt.lines().find_map(|l| l.strip_prefix("0::")).ok_or_else(|| {
        ErroSistema::invalido("/proc/self/cgroup", "sem linha 0:: (cgroup v1 puro?)")
    })?;
    let c = linha.trim().trim_end_matches(" (deleted)").to_string();
    if !c.starts_with('/') || c.split('/').any(|s| s == "..") || c.contains('\0') {
        return Err(ErroSistema::invalido("/proc/self/cgroup", "caminho suspeito"));
    }
    Ok(c)
}

/// O quê: lista de controladores (`cpu io memory pids`). Onde: cgroup.controllers.
pub fn parse_controladores(txt: &str) -> Vec<String> {
    txt.split_whitespace().map(sanear).filter(|s| !s.is_empty()).collect()
}

/// O quê: `ID`, `ID_LIKE` de os-release (aspas opcionais). Onde: coletor de distro.
pub fn parse_os_release(txt: &str) -> Result<(String, Vec<String>), ErroSistema> {
    let mut id = None;
    let mut like = Vec::new();
    for l in txt.lines() {
        if let Some((k, v)) = l.split_once('=') {
            let v = sanear(v.trim().trim_matches(|c| c == '"' || c == '\''));
            match k.trim() {
                "ID" => id = Some(v.to_lowercase()),
                "ID_LIKE" => like = v.split_whitespace().map(str::to_lowercase).collect(),
                _ => {}
            }
        }
    }
    id.filter(|s| !s.is_empty())
        .map(|i| (i, like))
        .ok_or_else(|| ErroSistema::invalido("os-release", "ID ausente"))
}

/// O quê: UID real da linha `Uid:` de /proc/self/status. Onde: coletor de kernel.
pub fn parse_uid(txt: &str) -> Result<u32, ErroSistema> {
    txt.lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| ErroSistema::invalido("/proc/self/status", "linha Uid: ausente"))
}

/// O quê: pares `Chave=Valor` (systemctl show). Onde: estado de serviços e ManagedOOM.
pub fn parse_show(txt: &str) -> HashMap<String, String> {
    txt.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (sanear(k), sanear(v))).collect()
}

/// O quê: entradas `(seção, chave, valor)` de um arquivo estilo INI do systemd; comentários
/// `#`/`;` ignorados. Onde: oomd.conf, system.conf, drop-ins.
pub fn parse_ini(txt: &str) -> Vec<(String, String, String)> {
    let mut sec = String::new();
    let mut out = Vec::new();
    for l in txt.lines().map(str::trim) {
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if let Some(s) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            sec = sanear(s);
        } else if let Some((k, v)) = l.split_once('=') {
            out.push((sec.clone(), sanear(k), sanear(v)));
        }
    }
    out
}
