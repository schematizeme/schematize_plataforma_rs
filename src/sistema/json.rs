//! Serialização JSON manual (sem serde): árvore [`J`] + trait [`ParaJson`] + impls do perfil.

use crate::sistema::tipos::*;

/// O quê: nó JSON. Onde: retorno de [`ParaJson::json`].
#[derive(Debug, Clone, PartialEq)]
pub enum J {
    /// null.
    Nulo,
    /// true/false.
    Bool(bool),
    /// Inteiro com sinal.
    Int(i64),
    /// Inteiro sem sinal.
    UInt(u64),
    /// Ponto flutuante (NaN/inf viram null).
    Float(f64),
    /// Texto.
    Str(String),
    /// Lista.
    Arr(Vec<J>),
    /// Objeto (ordem preservada).
    Obj(Vec<(String, J)>),
}

impl J {
    /// O quê: renderiza em texto JSON válido. Onde: `Perfil::to_json`.
    pub fn render(&self) -> String {
        let mut s = String::new();
        self.escrever(&mut s);
        s
    }

    /// O quê: escrita recursiva no buffer. Onde: `render`.
    fn escrever(&self, o: &mut String) {
        match self {
            J::Nulo => o.push_str("null"),
            J::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
            J::Int(n) => o.push_str(&n.to_string()),
            J::UInt(n) => o.push_str(&n.to_string()),
            J::Float(f) if f.is_finite() => o.push_str(&f.to_string()),
            J::Float(_) => o.push_str("null"),
            J::Str(t) => escapar(t, o),
            J::Arr(v) => {
                o.push('[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    x.escrever(o);
                }
                o.push(']');
            }
            J::Obj(v) => {
                o.push('{');
                for (i, (k, x)) in v.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    escapar(k, o);
                    o.push(':');
                    x.escrever(o);
                }
                o.push('}');
            }
        }
    }
}

/// O quê: escreve string JSON escapada (aspas, barra, controles incl. NUL, U+2028/9).
/// Onde: `J::escrever`.
fn escapar(t: &str, o: &mut String) {
    o.push('"');
    for c in t.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                o.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

/// O quê: tipo serializável à mão. Onde: todo tipo de `sistema::tipos`.
pub trait ParaJson {
    /// O quê: converte em árvore [`J`]. Onde: `render`/`Perfil::to_json`.
    fn json(&self) -> J;
}

/// O quê: monta objeto a partir de pares. Onde: impls abaixo.
fn obj(campos: Vec<(&str, J)>) -> J {
    J::Obj(campos.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

macro_rules! json_uint { ($($t:ty),*) => {$( impl ParaJson for $t { /// Inteiro sem sinal.
    fn json(&self) -> J { J::UInt(*self as u64) } } )*} }
macro_rules! json_int { ($($t:ty),*) => {$( impl ParaJson for $t { /// Inteiro com sinal.
    fn json(&self) -> J { J::Int(*self as i64) } } )*} }
json_uint!(u64, u32, usize);
json_int!(i64, i32);

impl ParaJson for bool {
    /// Booleano.
    fn json(&self) -> J {
        J::Bool(*self)
    }
}
impl ParaJson for f64 {
    /// Float.
    fn json(&self) -> J {
        J::Float(*self)
    }
}
impl ParaJson for String {
    /// Texto.
    fn json(&self) -> J {
        J::Str(self.clone())
    }
}
impl<T: ParaJson> ParaJson for Option<T> {
    /// `None` vira null.
    fn json(&self) -> J {
        self.as_ref().map(ParaJson::json).unwrap_or(J::Nulo)
    }
}
impl<T: ParaJson> ParaJson for Vec<T> {
    /// Lista.
    fn json(&self) -> J {
        J::Arr(self.iter().map(ParaJson::json).collect())
    }
}
impl<T: ParaJson> ParaJson for Lido<T> {
    /// `{valor, fonte, motivo}`.
    fn json(&self) -> J {
        obj(vec![
            ("valor", self.valor.json()),
            ("fonte", self.fonte.json()),
            ("motivo", self.motivo.json()),
        ])
    }
}
impl ParaJson for Limite {
    /// `"max"` ou número.
    fn json(&self) -> J {
        match self {
            Limite::Max => J::Str("max".into()),
            Limite::Valor(v) => J::UInt(*v),
        }
    }
}
impl ParaJson for Meminfo {
    /// Campos em KiB.
    fn json(&self) -> J {
        obj(vec![
            ("total_kib", self.total_kib.json()),
            ("disponivel_kib", self.disponivel_kib.json()),
            ("swap_total_kib", self.swap_total_kib.json()),
            ("swap_livre_kib", self.swap_livre_kib.json()),
        ])
    }
}
impl ParaJson for Zram {
    /// Dispositivo zram.
    fn json(&self) -> J {
        obj(vec![
            ("nome", self.nome.json()),
            ("disksize_bytes", self.disksize_bytes.json()),
            ("algoritmo", self.algoritmo.json()),
            ("prioridade", self.prioridade.json()),
        ])
    }
}
impl ParaJson for SwapDev {
    /// Linha de /proc/swaps.
    fn json(&self) -> J {
        obj(vec![
            ("arquivo", self.arquivo.json()),
            ("tamanho_kib", self.tamanho_kib.json()),
            ("usado_kib", self.usado_kib.json()),
            ("prioridade", self.prioridade.json()),
        ])
    }
}
impl ParaJson for Vm {
    /// sysctl vm.*.
    fn json(&self) -> J {
        obj(vec![
            ("swappiness", self.swappiness.json()),
            ("overcommit_memory", self.overcommit_memory.json()),
            ("min_free_kbytes", self.min_free_kbytes.json()),
            ("page_cluster", self.page_cluster.json()),
            ("vfs_cache_pressure", self.vfs_cache_pressure.json()),
            ("watermark_scale_factor", self.watermark_scale_factor.json()),
            ("oom_kill_allocating_task", self.oom_kill_allocating_task.json()),
        ])
    }
}
impl ParaJson for PsiLinha {
    /// avg10/60/300 + total.
    fn json(&self) -> J {
        obj(vec![
            ("avg10", self.avg10.json()),
            ("avg60", self.avg60.json()),
            ("avg300", self.avg300.json()),
            ("total_us", self.total_us.json()),
        ])
    }
}
impl ParaJson for PsiRecurso {
    /// some/full.
    fn json(&self) -> J {
        obj(vec![("some", self.some.json()), ("full", self.full.json())])
    }
}
impl ParaJson for Psi {
    /// memoria/cpu/io.
    fn json(&self) -> J {
        obj(vec![
            ("memoria", self.memoria.json()),
            ("cpu", self.cpu.json()),
            ("io", self.io.json()),
        ])
    }
}
impl ParaJson for NivelCgroup {
    /// Limites de um nível.
    fn json(&self) -> J {
        obj(vec![
            ("caminho", self.caminho.json()),
            ("memory_max", self.memory_max.json()),
            ("memory_high", self.memory_high.json()),
            ("swap_max", self.swap_max.json()),
            ("pids_max", self.pids_max.json()),
            ("memory_current", self.memory_current.json()),
            ("motivos", self.motivos.json()),
        ])
    }
}
impl ParaJson for EventosMem {
    /// memory.events.
    fn json(&self) -> J {
        obj(vec![
            ("low", self.low.json()),
            ("high", self.high.json()),
            ("max", self.max.json()),
            ("oom", self.oom.json()),
            ("oom_kill", self.oom_kill.json()),
        ])
    }
}
impl ParaJson for Efetivo {
    /// Limites efetivos.
    fn json(&self) -> J {
        obj(vec![
            ("memory_max", self.memory_max.json()),
            ("memory_high", self.memory_high.json()),
            ("swap_max", self.swap_max.json()),
            ("pids_max", self.pids_max.json()),
            ("folga_bytes", self.folga_bytes.json()),
        ])
    }
}
impl ParaJson for UserSlice {
    /// user@UID.service.
    fn json(&self) -> J {
        obj(vec![
            ("caminho", self.caminho.json()),
            ("delegados", self.delegados.json()),
            ("nivel", self.nivel.json()),
        ])
    }
}
impl ParaJson for Cgroup {
    /// Seção cgroup.
    fn json(&self) -> J {
        obj(vec![
            ("v2", self.v2.json()),
            ("controladores", self.controladores.json()),
            ("proprio", self.proprio.json()),
            ("niveis", self.niveis.json()),
            ("efetivo", self.efetivo.json()),
            ("eventos", self.eventos.json()),
            ("user_slice", self.user_slice.json()),
        ])
    }
}
impl ParaJson for Servico {
    /// Estado de serviço.
    fn json(&self) -> J {
        obj(vec![
            ("unidade", self.unidade.json()),
            ("ativo", self.ativo().json()),
            ("active_state", self.active_state.json()),
            ("sub_state", self.sub_state.json()),
            ("processo", self.processo.json()),
        ])
    }
}
impl ParaJson for OomdConf {
    /// oomd.conf.
    fn json(&self) -> J {
        obj(vec![
            ("swap_used_limit", self.swap_used_limit.json()),
            ("pressao_limite", self.pressao_limite.json()),
            ("pressao_duracao", self.pressao_duracao.json()),
        ])
    }
}
impl ParaJson for ManagedOom {
    /// ManagedOOM do user.slice.
    fn json(&self) -> J {
        obj(vec![("memoria", self.memoria.json()), ("swap", self.swap.json())])
    }
}
impl ParaJson for Accounting {
    /// Accounting padrão.
    fn json(&self) -> J {
        obj(vec![("memoria", self.memoria.json()), ("tarefas", self.tarefas.json())])
    }
}
impl ParaJson for Daemons {
    /// Seção daemons.
    fn json(&self) -> J {
        obj(vec![
            ("oomd", self.oomd.json()),
            ("earlyoom", self.earlyoom.json()),
            ("nohang", self.nohang.json()),
            ("ananicy", self.ananicy.json()),
            ("power_profiles", self.power_profiles.json()),
            ("oomd_conf", self.oomd_conf.json()),
            ("managed_oom_user", self.managed_oom_user.json()),
            ("accounting", self.accounting.json()),
        ])
    }
}
impl ParaJson for Kernel {
    /// Seção kernel.
    fn json(&self) -> J {
        obj(vec![
            ("versao", self.versao.json()),
            ("osrelease", self.osrelease.json()),
            ("distro_id", self.distro_id.json()),
            ("id_like", self.id_like.json()),
            ("cachyos", self.cachyos.json()),
            ("cpus", self.cpus.json()),
            ("uid", self.uid.json()),
        ])
    }
}
impl ParaJson for ProcessoOom {
    /// oom_score/adj.
    fn json(&self) -> J {
        obj(vec![
            ("oom_score", self.oom_score.json()),
            ("oom_score_adj", self.oom_score_adj.json()),
        ])
    }
}
impl ParaJson for Memoria {
    /// Seção memória.
    fn json(&self) -> J {
        obj(vec![
            ("meminfo", self.meminfo.json()),
            ("zram", self.zram.json()),
            ("swaps", self.swaps.json()),
            ("vm", self.vm.json()),
            ("psi", self.psi.json()),
        ])
    }
}
impl ParaJson for Gravidade {
    /// `"info"|"atencao"|"risco"`.
    fn json(&self) -> J {
        J::Str(
            match self {
                Gravidade::Info => "info",
                Gravidade::Atencao => "atencao",
                Gravidade::Risco => "risco",
            }
            .into(),
        )
    }
}
impl ParaJson for Achado {
    /// Achado bilíngue.
    fn json(&self) -> J {
        obj(vec![
            ("gravidade", self.gravidade.json()),
            ("codigo", self.codigo.json()),
            ("mensagem", self.mensagem.json()),
            ("sugestao", self.sugestao.json()),
            ("mensagem_en", self.mensagem_en.json()),
            ("sugestao_en", self.sugestao_en.json()),
        ])
    }
}
impl ParaJson for Perfil {
    /// Perfil inteiro.
    fn json(&self) -> J {
        obj(vec![
            ("cgroup", self.cgroup.json()),
            ("memoria", self.memoria.json()),
            ("daemons", self.daemons.json()),
            ("kernel", self.kernel.json()),
            ("processo", self.processo.json()),
            ("achados", self.achados.json()),
        ])
    }
}
impl ParaJson for MargemMemoria {
    /// Margem em MiB.
    fn json(&self) -> J {
        obj(vec![
            ("reservavel_mib", self.reservavel_mib.json()),
            ("disponivel_host_mib", self.disponivel_host_mib.json()),
            ("piso_critico_mib", self.piso_critico_mib.json()),
            ("folga_cgroup_mib", self.folga_cgroup_mib.json()),
            ("limitado_por", J::Str(format!("{:?}", self.limitado_por).to_lowercase())),
        ])
    }
}
