//! Coleta dos daemons de OOM/recursos (systemd-oomd, earlyoom, nohang, ananicy-cpp,
//! power-profiles) e da config do systemd. Só leitura: `systemctl show` com argv puro e timeout.

use crate::sistema::fonte::Fonte;
use crate::sistema::parse::*;
use crate::sistema::tipos::*;
use std::time::Duration;

/// Prazo para qualquer `systemctl show`.
const PRAZO: Duration = Duration::from_secs(2);

/// O quê: nomes `comm` de todos os processos (/proc/N/comm). Onde: `coletar`.
pub fn comms_em_execucao(f: &dyn Fonte) -> Lido<Vec<String>> {
    match f.listar_dir("/proc") {
        Ok(ents) => {
            let v = ents
                .iter()
                .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit()))
                .filter_map(|e| f.ler_arquivo(&format!("/proc/{e}/comm")).ok())
                .map(|t| sanear(&t))
                .collect();
            Lido::ok(v, "/proc/*/comm")
        }
        Err(e) => Lido::falha("/proc/*/comm", e),
    }
}

/// O quê: `comm` bate com o nome (o kernel trunca em 15 chars). Onde: `servico`.
pub fn comm_confere(comm: &str, nome: &str) -> bool {
    comm == nome || (comm.len() == 15 && nome.starts_with(comm))
}

/// O quê: estado de uma unit (systemctl) + presença de processo. Onde: `coletar`.
pub fn servico(f: &dyn Fonte, unidade: &str, comm: &str, comms: &Lido<Vec<String>>) -> Servico {
    let fonte = format!("systemctl show -p ActiveState,SubState {unidade}");
    let r = f.rodar_cmd("systemctl", &["show", "-p", "ActiveState,SubState", unidade], PRAZO);
    let (active_state, sub_state) = match r {
        Ok(t) => {
            let m = parse_show(&t);
            let a = m.get("ActiveState").cloned().filter(|s| !s.is_empty());
            let s = m.get("SubState").cloned();
            (
                a.map(|a| Lido::ok(a, &fonte))
                    .unwrap_or_else(|| Lido::falha(&fonte, "ActiveState ausente")),
                s,
            )
        }
        Err(e) => (Lido::falha(&fonte, e), None),
    };
    let processo = match comms.get() {
        Some(v) => Lido::ok(v.iter().any(|c| comm_confere(c, comm)), &comms.fonte),
        None => Lido::falha(&comms.fonte, comms.motivo.clone().unwrap_or_default()),
    };
    Servico { unidade: unidade.to_string(), active_state, sub_state, processo }
}

/// O quê: lê arquivo principal + drop-ins `*.conf` (ordem: dirs dados, depois o último vence).
/// `principal` vazio = só drop-ins. Onde: oomd.conf, system.conf e ManagedOOM.
pub fn ler_conf(
    f: &dyn Fonte,
    principal: &str,
    dirs: &[&str],
) -> Result<Vec<(String, String, String)>, crate::sistema::erro::ErroSistema> {
    let mut e: Vec<(String, String, String)> = Vec::new();
    let mut leu = false;
    if let Ok(t) = f.ler_arquivo(principal) {
        e.extend(parse_ini(&t));
        leu = true;
    }
    for d in dirs {
        for nome in f.listar_dir(d).unwrap_or_default().iter().filter(|n| n.ends_with(".conf")) {
            if let Ok(t) = f.ler_arquivo(&format!("{d}/{nome}")) {
                e.extend(parse_ini(&t));
                leu = true;
            }
        }
    }
    if leu {
        Ok(e)
    } else {
        Err(crate::sistema::erro::ErroSistema::Ausente(if principal.is_empty() {
            "drop-ins".to_string()
        } else {
            principal.to_string()
        }))
    }
}

/// O quê: último valor de `secao.chave` entre as entradas. Onde: coletores de config.
fn ultimo(e: &[(String, String, String)], sec: &str, chave: &str) -> Option<String> {
    e.iter()
        .rev()
        .find(|(s, k, v)| s == sec && k == chave && !v.is_empty())
        .map(|(_, _, v)| v.clone())
}

/// O quê: config efetiva do systemd-oomd. Onde: `coletar`.
pub fn oomd_conf(f: &dyn Fonte) -> Lido<OomdConf> {
    let r = ler_conf(
        f,
        "/etc/systemd/oomd.conf",
        &["/usr/lib/systemd/oomd.conf.d", "/run/systemd/oomd.conf.d", "/etc/systemd/oomd.conf.d"],
    );
    Lido::de(
        "/etc/systemd/oomd.conf(.d)",
        r.map(|e| OomdConf {
            swap_used_limit: ultimo(&e, "OOM", "SwapUsedLimit"),
            pressao_limite: ultimo(&e, "OOM", "DefaultMemoryPressureLimit"),
            pressao_duracao: ultimo(&e, "OOM", "DefaultMemoryPressureDurationSec"),
        }),
    )
}

/// O quê: DefaultMemoryAccounting/DefaultTasksAccounting. Onde: `coletar`.
pub fn accounting(f: &dyn Fonte) -> Lido<Accounting> {
    let r = ler_conf(
        f,
        "/etc/systemd/system.conf",
        &[
            "/usr/lib/systemd/system.conf.d",
            "/run/systemd/system.conf.d",
            "/etc/systemd/system.conf.d",
        ],
    );
    Lido::de(
        "/etc/systemd/system.conf(.d)",
        r.map(|e| Accounting {
            memoria: ultimo(&e, "Manager", "DefaultMemoryAccounting"),
            tarefas: ultimo(&e, "Manager", "DefaultTasksAccounting"),
        }),
    )
}

/// O quê: ManagedOOM via `systemctl show` (manager do sistema, depois `--user`), ou via drop-in.
/// Onde: `coletar`. O `user.slice` que o oomd supervisiona é o do manager do sistema.
pub fn managed_oom_user(f: &dyn Fonte) -> Lido<ManagedOom> {
    let props = "ManagedOOMMemoryPressure,ManagedOOMSwap";
    let tentativas: [&[&str]; 2] =
        [&["show", "-p", props, "user.slice"], &["--user", "show", "-p", props, "user.slice"]];
    for args in tentativas {
        if let Ok(t) = f.rodar_cmd("systemctl", args, PRAZO) {
            let m = parse_show(&t);
            if m.contains_key("ManagedOOMMemoryPressure") || m.contains_key("ManagedOOMSwap") {
                let v = ManagedOom {
                    memoria: m.get("ManagedOOMMemoryPressure").cloned(),
                    swap: m.get("ManagedOOMSwap").cloned(),
                };
                return Lido::ok(v, &format!("systemctl {}", args.join(" ")));
            }
        }
    }
    let r = ler_conf(
        f,
        "",
        &[
            "/usr/lib/systemd/system/user.slice.d",
            "/usr/lib/systemd/system/user-.slice.d",
            "/run/systemd/system/user.slice.d",
            "/etc/systemd/system/user.slice.d",
        ],
    );
    Lido::de(
        "user.slice.d/*.conf",
        r.map(|e| ManagedOom {
            memoria: ultimo(&e, "Slice", "ManagedOOMMemoryPressure"),
            swap: ultimo(&e, "Slice", "ManagedOOMSwap"),
        }),
    )
}

/// O quê: coleta completa dos daemons. Onde: `identificar`.
pub fn coletar(f: &dyn Fonte) -> Daemons {
    let comms = comms_em_execucao(f);
    Daemons {
        oomd: servico(f, "systemd-oomd", "systemd-oomd", &comms),
        earlyoom: servico(f, "earlyoom", "earlyoom", &comms),
        nohang: servico(f, "nohang", "nohang", &comms),
        ananicy: servico(f, "ananicy-cpp", "ananicy-cpp", &comms),
        power_profiles: servico(f, "power-profiles-daemon", "power-profiles-daemon", &comms),
        oomd_conf: oomd_conf(f),
        managed_oom_user: managed_oom_user(f),
        accounting: accounting(f),
    }
}
