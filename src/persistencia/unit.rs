//! Unit de usuário do systemd para o vigia, gerada como TEXTO. Nada é instalado no sistema.

use super::vigia::VigiaCfg;

/// **O quê:** cita um argumento no formato do `ExecStart=` (aspas duplas, `\`, `"` e `%`).
///
/// **Onde:** `unit_systemd`.
fn cita(a: &str) -> String {
    let e = a.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    format!("\"{e}\"")
}

/// **O quê:** gera o texto da slice + service do vigia (dois arquivos, separados por marcadores
/// `# --- <caminho> ---`). `Restart=always`, `OOMPolicy=continue`, `MemoryMax` folgado no slice.
///
/// **Onde:** `schematize-vigia --print-unit` / instalador do ops (que decide onde gravar).
/// Convenção do `ExecStart`: `<exe_vigia> --estado <dir> -- <comando> <args...>`.
/// `OOMScoreAdjust` NÃO entra na unit (negativo exige CAP_SYS_RESOURCE, ausente no manager de
/// usuário); o vigia faz a tentativa best-effort em `/proc` por conta própria.
pub fn unit_systemd(cfg: &VigiaCfg) -> String {
    let mut exec = vec![cita(&cfg.exe_vigia.to_string_lossy())];
    exec.extend([
        "--estado".to_string(),
        cita(&cfg.dir_estado.to_string_lossy()),
        "--".to_string(),
    ]);
    exec.push(cita(&cfg.comando.to_string_lossy()));
    exec.extend(cfg.args.iter().map(|a| cita(a)));
    format!(
        "# --- ~/.config/systemd/user/schematize-vigia.slice ---\n\
[Unit]\nDescription=Slice do vigia schematize\n\n\
[Slice]\nMemoryMax={mem}\nMemorySwapMax=0\n\n\
# --- ~/.config/systemd/user/schematize-vigia.service ---\n\
[Unit]\nDescription=Vigia schematize (relança o orquestrador com --resume)\n\
StartLimitIntervalSec=0\n\n\
[Service]\nType=simple\nSlice=schematize-vigia.slice\nExecStart={exec}\n\
Restart=always\nRestartSec=2\nOOMPolicy=continue\nNoNewPrivileges=yes\nPrivateTmp=yes\n\n\
[Install]\nWantedBy=default.target\n",
        mem = cfg.memoria_max_slice,
        exec = exec.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit traz as diretivas exigidas e cita argumentos com espaço/%.
    #[test]
    fn diretivas_e_citacao() {
        let mut cfg = VigiaCfg::novo("/usr/bin/orq", "/var/estado");
        cfg.args = vec!["a b".into(), "50%".into()];
        let u = unit_systemd(&cfg);
        for d in ["Restart=always", "OOMPolicy=continue", "MemoryMax=4G", "\"a b\"", "\"50%%\""] {
            assert!(u.contains(d), "faltou {d}\n{u}");
        }
    }
}
