//! `Journal`: WAL append-only em segmentos, com fsync configurável, escritor único (flock),
//! recuperação que trunca cauda corrompida e compactação a partir de snapshot.

use super::erro::{io, Erro};
use super::fsutil::{sincroniza_dir, Trava};
use super::registro::{codificar, Registro, MAX_PAYLOAD};
use super::segmento::{listar, nome, varrer};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// **O quê:** política de `fsync` por anexo.
///
/// **Onde:** `Config`. `Cada` é o default e o único que garante que "anexar retornou Ok" ==
/// "sobrevive a queda de energia"; `Lote` e `Nunca` trocam durabilidade por vazão.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durabilidade {
    /// `fsync` a cada registro.
    Cada,
    /// `fsync` a cada `n` registros ou quando passaram `ms` desde o último (checado no anexo).
    Lote { n: usize, ms: u64 },
    /// Nunca por conta própria (só em rotação e em `sincronizar`).
    Nunca,
}

/// **O quê:** configuração do journal. **Onde:** `Journal::abrir`.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub durabilidade: Durabilidade,
    /// Tamanho a partir do qual o segmento rotaciona (default 4 MiB).
    pub tamanho_segmento: u64,
}

impl Default for Config {
    /// **O quê:** `Cada` + segmentos de 4 MiB. **Onde:** `Config::default()`.
    fn default() -> Self {
        Config { durabilidade: Durabilidade::Cada, tamanho_segmento: 4 << 20 }
    }
}

/// **O quê:** o que a recuperação encontrou e fez.
///
/// **Onde:** devolvido por `recuperar`/`recuperar_desde`; vai para log/alerta de quem retoma.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relatorio {
    pub validos: u64,
    pub bytes_descartados: u64,
    pub motivo: Option<String>,
    pub segmentos: usize,
    pub segmentos_removidos: usize,
    pub primeiro_seq: Option<u64>,
    pub ultimo_seq: Option<u64>,
    pub lock_orfao: bool,
    /// Aviso sobre o snapshot (preenchido pela `Maquina`): usou `.prev` ou replay do zero.
    pub snapshot_aviso: Option<String>,
}

/// Segmento aberto para anexar.
struct Ativo {
    arquivo: File,
    tam: u64,
}

/// **O quê:** o journal (um único escritor por diretório).
///
/// **Onde:** base da `Maquina` e do estado do `Vigia`.
pub struct Journal {
    dir: PathBuf,
    cfg: Config,
    _trava: Trava,
    lock_orfao: bool,
    ativo: Option<Ativo>,
    proximo: u64,
    pendentes: usize,
    ultimo_sync: Instant,
    envenenado: Option<String>,
}

impl Journal {
    /// **O quê:** cria o diretório e toma o lock exclusivo; NÃO lê segmentos ainda.
    ///
    /// **Onde:** início de todo processo escritor. Segundo escritor: `Erro::LockOcupado`.
    /// Chame `recuperar` antes de `anexar`.
    pub fn abrir(dir: &Path, cfg: Config) -> Result<Journal, Erro> {
        if cfg.tamanho_segmento == 0 {
            return Err(Erro::Config("tamanho_segmento deve ser > 0".into()));
        }
        std::fs::create_dir_all(dir).map_err(io(format!("criar {}", dir.display())))?;
        let (trava, lock_orfao) = Trava::adquirir(&dir.join("journal.lock"))?;
        Ok(Journal {
            dir: dir.to_path_buf(),
            cfg,
            _trava: trava,
            lock_orfao,
            ativo: None,
            proximo: 1,
            pendentes: 0,
            ultimo_sync: Instant::now(),
            envenenado: None,
        })
    }

    /// **O quê:** seq que o próximo `anexar` receberá. **Onde:** `Maquina`, diagnósticos.
    pub fn proximo_seq(&self) -> u64 {
        self.proximo
    }

    /// **O quê:** recupera tudo (equivale a `recuperar_desde(0)`). **Onde:** testes, vigia.
    pub fn recuperar(&mut self) -> Result<(Vec<Registro>, Relatorio), Erro> {
        self.recuperar_desde(0)
    }

    /// **O quê:** varre os segmentos, valida CRC e seq, TRUNCA a cauda parcial/corrompida do
    /// último segmento e devolve os registros com `seq >= desde` + relatório.
    ///
    /// **Onde:** boot de quem retoma. Hostil/truncado/zerado/lixo nunca causa pânico; buraco de
    /// seq e corrupção em segmento não-final são erros explícitos (nada é apagado nesses casos).
    pub fn recuperar_desde(&mut self, desde: u64) -> Result<(Vec<Registro>, Relatorio), Erro> {
        let segs = listar(&self.dir)?;
        let mut rel =
            Relatorio { segmentos: segs.len(), lock_orfao: self.lock_orfao, ..Default::default() };
        let (mut saida, mut esperado, mut ativo) = (Vec::new(), None::<u64>, None);
        for (i, (primeiro, path)) in segs.iter().enumerate() {
            let ultimo = i + 1 == segs.len();
            let buf = std::fs::read(path).map_err(io(format!("ler {}", path.display())))?;
            let inicio = esperado.unwrap_or(*primeiro);
            let v = varrer(&buf, inicio, path)?;
            if let Some(motivo) = v.ruim {
                if !ultimo {
                    let offset = v.bytes_ok as u64;
                    return Err(Erro::CorrupcaoNoMeio { segmento: path.clone(), offset, motivo });
                }
                trunca(path, v.bytes_ok as u64)?;
                rel.bytes_descartados += (buf.len() - v.bytes_ok) as u64;
                rel.motivo = Some(motivo);
            }
            if v.registros.is_empty() && !(ultimo && inicio == *primeiro) {
                std::fs::remove_file(path).map_err(io(format!("remover {}", path.display())))?;
                rel.segmentos_removidos += 1;
                continue;
            }
            esperado = Some(inicio + v.registros.len() as u64);
            rel.validos += v.registros.len() as u64;
            saida.extend(v.registros.into_iter().filter(|r| r.seq >= desde));
            if ultimo {
                ativo = Some((path.clone(), v.bytes_ok as u64));
            }
        }
        self.finaliza_recuperacao(esperado, ativo, &mut rel)?;
        Ok((saida, rel))
    }

    /// **O quê:** abre (ou cria) o segmento ativo e fixa `proximo`/relatório após a varredura.
    ///
    /// **Onde:** fim de `recuperar_desde`.
    fn finaliza_recuperacao(
        &mut self,
        esperado: Option<u64>,
        ativo: Option<(PathBuf, u64)>,
        rel: &mut Relatorio,
    ) -> Result<(), Erro> {
        self.proximo = esperado.unwrap_or(1);
        rel.ultimo_seq = (rel.validos > 0).then(|| self.proximo - 1);
        rel.primeiro_seq = rel.ultimo_seq.map(|u| u + 1 - rel.validos);
        self.ativo = Some(match ativo {
            Some((path, tam)) => Ativo { arquivo: abre_append(&path)?, tam },
            None => self.cria_segmento(self.proximo)?,
        });
        if rel.segmentos_removidos > 0 || rel.motivo.is_some() {
            sincroniza_dir(&self.dir)?;
        }
        self.envenenado = None;
        self.pendentes = 0;
        Ok(())
    }

    /// **O quê:** cria o arquivo do segmento que começa em `primeiro` e sincroniza o diretório.
    ///
    /// **Onde:** primeira abertura, rotação, `pular_para`.
    fn cria_segmento(&self, primeiro: u64) -> Result<Ativo, Erro> {
        let path = self.dir.join(nome(primeiro));
        let arquivo = abre_append(&path)?;
        sincroniza_dir(&self.dir)?;
        Ok(Ativo { arquivo, tam: 0 })
    }

    /// **O quê:** anexa um registro e devolve seu seq.
    ///
    /// **Onde:** toda transição de estado. Falha de escrita (ex.: disco cheio) desfaz a escrita
    /// parcial (truncate ao tamanho anterior) e propaga o erro com o estado anterior intacto;
    /// falha de fsync ou de rollback deixa o journal `Envenenado` (reabra e recupere).
    pub fn anexar(&mut self, tipo: u8, payload: &[u8]) -> Result<u64, Erro> {
        if let Some(m) = &self.envenenado {
            return Err(Erro::Envenenado(m.clone()));
        }
        if payload.len() > MAX_PAYLOAD {
            return Err(Erro::PayloadGrande { tam: payload.len(), max: MAX_PAYLOAD });
        }
        let rec = codificar(self.proximo, tipo, payload);
        self.rotaciona_se_preciso(rec.len() as u64)?;
        let seq = self.proximo;
        self.escreve(&rec)?;
        self.proximo += 1;
        self.aplica_durabilidade()?;
        Ok(seq)
    }

    /// **O quê:** rotaciona o segmento se o registro estourar o tamanho (e o segmento não
    /// estiver vazio). O segmento antigo é sincronizado antes: só o último pode ter cauda torn.
    ///
    /// **Onde:** `anexar`.
    fn rotaciona_se_preciso(&mut self, extra: u64) -> Result<(), Erro> {
        let limite = self.cfg.tamanho_segmento;
        let tam = self.ativo.as_ref().ok_or(Erro::NaoRecuperado)?.tam;
        if tam == 0 || tam + extra <= limite {
            return Ok(());
        }
        self.sincronizar()?;
        let novo = self.cria_segmento(self.proximo)?;
        self.ativo = Some(novo);
        Ok(())
    }

    /// **O quê:** escreve `rec` no segmento ativo; em falha, desfaz o parcial.
    ///
    /// **Onde:** `anexar`.
    fn escreve(&mut self, rec: &[u8]) -> Result<(), Erro> {
        let ativo = self.ativo.as_mut().ok_or(Erro::NaoRecuperado)?;
        match ativo.arquivo.write_all(rec) {
            Ok(()) => {
                ativo.tam += rec.len() as u64;
                Ok(())
            }
            Err(e) => match ativo.arquivo.set_len(ativo.tam) {
                Ok(()) => {
                    Err(Erro::Io { contexto: "anexar ao journal (revertido)".into(), fonte: e })
                }
                Err(r) => {
                    let m = format!("escrita falhou ({e}) e o rollback também ({r})");
                    self.envenenado = Some(m.clone());
                    Err(Erro::Envenenado(m))
                }
            },
        }
    }

    /// **O quê:** aplica a política de fsync após um anexo.
    ///
    /// **Onde:** `anexar`.
    fn aplica_durabilidade(&mut self) -> Result<(), Erro> {
        self.pendentes += 1;
        let agora = match self.cfg.durabilidade {
            Durabilidade::Cada => true,
            Durabilidade::Nunca => false,
            Durabilidade::Lote { n, ms } => {
                self.pendentes >= n || self.ultimo_sync.elapsed() >= Duration::from_millis(ms)
            }
        };
        if agora {
            self.sincronizar()
        } else {
            Ok(())
        }
    }

    /// **O quê:** `fsync` do segmento ativo; falha envenena o journal (estado de página incerto).
    ///
    /// **Onde:** `Lote`/`Nunca` quando o chamador quer um ponto de durabilidade; rotação; snapshot.
    pub fn sincronizar(&mut self) -> Result<(), Erro> {
        let ativo = self.ativo.as_ref().ok_or(Erro::NaoRecuperado)?;
        if let Err(e) = ativo.arquivo.sync_data() {
            self.envenenado = Some(format!("fsync falhou: {e}"));
            return Err(Erro::Io { contexto: "fsync do journal".into(), fonte: e });
        }
        self.pendentes = 0;
        self.ultimo_sync = Instant::now();
        Ok(())
    }

    /// **O quê:** sincroniza e solta o lock. **Onde:** shutdown gracioso.
    pub fn fechar(mut self) -> Result<(), Erro> {
        if self.ativo.is_some() && self.envenenado.is_none() {
            self.sincronizar()?;
        }
        Ok(())
    }

    /// **O quê:** remove segmentos inativos inteiramente cobertos por `ate_seq` (todos os seus
    /// registros têm seq <= `ate_seq`). Devolve quantos removeu.
    ///
    /// **Onde:** `Maquina::snapshot`, depois do snapshot estar durável.
    pub fn compactar(&mut self, ate_seq: u64) -> Result<usize, Erro> {
        let segs = listar(&self.dir)?;
        let mut removidos = 0;
        for par in segs.windows(2) {
            if par[1].0 <= ate_seq.saturating_add(1) {
                let p = &par[0].1;
                std::fs::remove_file(p).map_err(io(format!("remover {}", p.display())))?;
                removidos += 1;
            }
        }
        if removidos > 0 {
            sincroniza_dir(&self.dir)?;
        }
        Ok(removidos)
    }

    /// **O quê:** descarta TODO o conteúdo e recomeça a numeração em `seq` (>= `proximo`).
    ///
    /// **Onde:** `Maquina::abrir` quando o snapshot está à frente do journal (cauda perdida).
    /// O chamador garante que o snapshot cobre tudo que está sendo descartado.
    pub fn pular_para(&mut self, seq: u64) -> Result<(), Erro> {
        if seq < self.proximo {
            return Err(Erro::Config(format!("pular_para({seq}) < proximo ({})", self.proximo)));
        }
        self.ativo = None;
        for (_, p) in listar(&self.dir)? {
            std::fs::remove_file(&p).map_err(io(format!("remover {}", p.display())))?;
        }
        self.proximo = seq;
        self.ativo = Some(self.cria_segmento(seq)?);
        self.pendentes = 0;
        Ok(())
    }
}

/// **O quê:** abre `path` para anexar (cria se faltar). **Onde:** ativo e rotação.
fn abre_append(path: &Path) -> Result<File, Erro> {
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(io(format!("abrir segmento {}", path.display())))
}

/// **O quê:** trunca `path` em `tam` bytes e sincroniza.
///
/// **Onde:** descarte da cauda torn na recuperação.
fn trunca(path: &Path, tam: u64) -> Result<(), Erro> {
    let f = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io(format!("abrir para truncar {}", path.display())))?;
    f.set_len(tam).map_err(io(format!("truncar {}", path.display())))?;
    f.sync_all().map_err(io(format!("fsync após truncar {}", path.display())))
}
