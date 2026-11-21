//! Cálculo da margem segura de memória para o orquestrador.

use crate::sistema::tipos::*;

/// Piso mínimo preservado do host, em MiB.
const PISO_MIN_MIB: u64 = 512;

/// O quê: quanto o orquestrador pode reservar sem tocar o MemAvailable crítico (maior entre
/// 512 MiB e 10% de MemTotal) e sem passar da folga do teto de cgroup efetivo.
/// Onde: orçamento de memória (`memoria`) e escalonador de tarefas.
pub fn margem_segura(p: &Perfil) -> MargemMemoria {
    let mi = p.memoria.meminfo.get();
    let total_mib = mi.and_then(|m| m.total_kib).unwrap_or(0) / 1024;
    let disp = mi.and_then(|m| m.disponivel_kib).map(|k| k / 1024);
    let piso = PISO_MIN_MIB.max(total_mib / 10);
    let folga = p.cgroup.efetivo.folga_bytes.map(|b| b >> 20);
    let (reservavel, por) = match disp {
        None => (0, LimitadoPor::Desconhecido),
        Some(d) => {
            let host = d.saturating_sub(piso);
            match folga {
                Some(f) if f < host => (f, LimitadoPor::Cgroup),
                _ => (host, LimitadoPor::Host),
            }
        }
    };
    MargemMemoria {
        reservavel_mib: reservavel,
        disponivel_host_mib: disp.unwrap_or(0),
        piso_critico_mib: piso,
        folga_cgroup_mib: folga,
        limitado_por: por,
    }
}
