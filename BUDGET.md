# BUDGET — schematize_plataforma v0.4.0

Orçamentos medidos (Linux x86_64, `cargo build --release --features persistencia`, perfil release **padrão**,
sem `[profile.release]` próprio neste repo).

| item | orçamento | medido |
|---|---|---|
| RSS ocioso do `schematize-vigia` (vigiando `sleep`) | < 5 MiB | **2,7 MiB** (VmRSS = VmHWM = 2732 kB) |
| Tamanho do binário `schematize-vigia` | informativo | 654000 B (639 KiB) com símbolos; 502776 B (491 KiB) após `strip` |
| Dependências do bin | 0 | 0 (só std) |

Como reproduzir: `schematize-vigia --estado /tmp/vg -- sleep 4 &` e `grep -E 'VmRSS|VmHWM' /proc/<pid>/status`.
O teste `tests/persistencia_vigia.rs` também guarda VmHWM < 5 MiB do vigia como biblioteca.
