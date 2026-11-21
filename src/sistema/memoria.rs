//! Coleta de memória do host: meminfo, zram, swaps, sysctl vm.* e PSI. Somente leitura.

use crate::sistema::fonte::Fonte;
use crate::sistema::parse::*;
use crate::sistema::tipos::*;

/// O quê: lê um sysctl numérico de /proc/sys/vm. Onde: `coletar_vm`.
fn sysctl(f: &dyn Fonte, nome: &str) -> Lido<i64> {
    let p = format!("/proc/sys/vm/{nome}");
    Lido::de(&p, f.ler_arquivo(&p).and_then(|t| parse_i64(&t, nome)))
}

/// O quê: os sete sysctl vm.* pedidos. Onde: `coletar`.
pub fn coletar_vm(f: &dyn Fonte) -> Vm {
    Vm {
        swappiness: sysctl(f, "swappiness"),
        overcommit_memory: sysctl(f, "overcommit_memory"),
        min_free_kbytes: sysctl(f, "min_free_kbytes"),
        page_cluster: sysctl(f, "page-cluster"),
        vfs_cache_pressure: sysctl(f, "vfs_cache_pressure"),
        watermark_scale_factor: sysctl(f, "watermark_scale_factor"),
        oom_kill_allocating_task: sysctl(f, "oom_kill_allocating_task"),
    }
}

/// O quê: PSI de um recurso. Onde: `coletar`.
fn psi_de(f: &dyn Fonte, nome: &str) -> Lido<PsiRecurso> {
    let p = format!("/proc/pressure/{nome}");
    Lido::de(&p, f.ler_arquivo(&p).and_then(|t| parse_psi(&t)))
}

/// O quê: nome de dispositivo zram válido (`zram` + dígitos). Onde: `coletar_zram`.
fn nome_zram_ok(n: &str) -> bool {
    n.strip_prefix("zram")
        .map(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or(false)
}

/// O quê: lê um dispositivo zram (disksize, algoritmo, prioridade). Onde: `coletar_zram`.
fn zram_um(f: &dyn Fonte, nome: &str, swaps: &[SwapDev]) -> Zram {
    let base = format!("/sys/block/{nome}");
    let disksize = f
        .ler_arquivo(&format!("{base}/disksize"))
        .ok()
        .and_then(|t| parse_u64(&t, "disksize").ok());
    let algoritmo = f
        .ler_arquivo(&format!("{base}/comp_algorithm"))
        .ok()
        .and_then(|t| parse_algoritmo_zram(&t));
    let dev = format!("/dev/{nome}");
    Zram {
        nome: nome.to_string(),
        disksize_bytes: disksize,
        algoritmo,
        prioridade: swaps.iter().find(|s| s.arquivo == dev).map(|s| s.prioridade),
    }
}

/// O quê: lista zram* em /sys/block. Onde: `coletar`.
pub fn coletar_zram(f: &dyn Fonte, swaps: &[SwapDev]) -> Lido<Vec<Zram>> {
    match f.listar_dir("/sys/block") {
        Ok(nomes) => Lido::ok(
            nomes.iter().filter(|n| nome_zram_ok(n)).map(|n| zram_um(f, n, swaps)).collect(),
            "/sys/block/zram*",
        ),
        Err(e) => Lido::falha("/sys/block/zram*", e),
    }
}

/// O quê: coleta completa de memória. Onde: `identificar`.
pub fn coletar(f: &dyn Fonte) -> Memoria {
    let meminfo =
        Lido::de("/proc/meminfo", f.ler_arquivo("/proc/meminfo").and_then(|t| parse_meminfo(&t)));
    let swaps = Lido::de("/proc/swaps", f.ler_arquivo("/proc/swaps").map(|t| parse_swaps(&t)));
    let zram = coletar_zram(f, swaps.get().map(Vec::as_slice).unwrap_or(&[]));
    Memoria {
        meminfo,
        zram,
        swaps,
        vm: coletar_vm(f),
        psi: Psi { memoria: psi_de(f, "memory"), cpu: psi_de(f, "cpu"), io: psi_de(f, "io") },
    }
}
