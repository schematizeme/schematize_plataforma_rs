//! Costura de leitura do host: trait [`Fonte`] + implementação real ([`FonteReal`], somente
//! leitura, sem sudo) + falsa para testes ([`FonteFalsa`]).

use crate::sistema::erro::ErroSistema;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Teto de bytes lidos por arquivo (defesa contra arquivo gigante/hostil).
pub const LIMITE_BYTES_ARQUIVO: u64 = 1 << 20;
/// Teto de entradas devolvidas por listagem de diretório.
pub const LIMITE_ENTRADAS_DIR: usize = 4096;

/// O quê: tudo o que o detector precisa do host, injetável.
/// Onde: `identificar(&dyn Fonte)`; produção usa [`FonteReal`], testes [`FonteFalsa`].
pub trait Fonte {
    /// O quê: lê um arquivo como texto (UTF-8 com substituição). Onde: todos os coletores.
    fn ler_arquivo(&self, caminho: &str) -> Result<String, ErroSistema>;
    /// O quê: lista nomes (não caminhos) de um diretório, ordenados. Onde: /proc, drop-ins, zram.
    fn listar_dir(&self, caminho: &str) -> Result<Vec<String>, ErroSistema>;
    /// O quê: roda `prog args` (argv puro, sem shell) e devolve stdout; mata ao estourar `timeout`.
    /// Onde: `systemctl show` (somente leitura).
    fn rodar_cmd(
        &self,
        prog: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, ErroSistema>;
    /// O quê: número de CPUs lógicas visíveis. Onde: coletor de kernel.
    fn cpus(&self) -> Option<usize>;
}

/// O quê: [`Fonte`] que lê o host de verdade. Onde: `identificar_host()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FonteReal;

impl Fonte for FonteReal {
    /// O quê: `read` limitado a [`LIMITE_BYTES_ARQUIVO`]. Onde: ver trait.
    fn ler_arquivo(&self, caminho: &str) -> Result<String, ErroSistema> {
        let f = std::fs::File::open(caminho).map_err(|e| ErroSistema::de_io(caminho, &e))?;
        let mut buf = Vec::new();
        f.take(LIMITE_BYTES_ARQUIVO)
            .read_to_end(&mut buf)
            .map_err(|e| ErroSistema::de_io(caminho, &e))?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// O quê: `read_dir` ordenado e limitado. Onde: ver trait.
    fn listar_dir(&self, caminho: &str) -> Result<Vec<String>, ErroSistema> {
        let rd = std::fs::read_dir(caminho).map_err(|e| ErroSistema::de_io(caminho, &e))?;
        let mut nomes: Vec<String> = rd
            .filter_map(Result::ok)
            .take(LIMITE_ENTRADAS_DIR)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        nomes.sort();
        Ok(nomes)
    }

    /// O quê: spawn com stdout em thread leitora + espera com prazo. Onde: ver trait.
    fn rodar_cmd(
        &self,
        prog: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, ErroSistema> {
        let mut filho = Command::new(prog)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ErroSistema::de_io(prog, &e))?;
        let mut saida =
            filho.stdout.take().ok_or_else(|| ErroSistema::Io(format!("{prog}: sem stdout")))?;
        let leitor = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = (&mut saida).take(LIMITE_BYTES_ARQUIVO).read_to_end(&mut b);
            b
        });
        let ini = Instant::now();
        loop {
            match filho.try_wait() {
                Ok(Some(st)) => {
                    let b = leitor.join().unwrap_or_default();
                    return if st.success() {
                        Ok(String::from_utf8_lossy(&b).into_owned())
                    } else {
                        Err(ErroSistema::Io(format!("{prog}: status {st}")))
                    };
                }
                Ok(None) if ini.elapsed() >= timeout => {
                    let _ = filho.kill();
                    let _ = filho.wait();
                    let _ = leitor.join();
                    return Err(ErroSistema::Timeout(prog.to_string()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(ErroSistema::de_io(prog, &e)),
            }
        }
    }

    /// O quê: `available_parallelism`. Onde: ver trait.
    fn cpus(&self) -> Option<usize> {
        std::thread::available_parallelism().ok().map(|n| n.get())
    }
}

/// O quê: host de mentira montado à mão (arquivos, saídas de comando, nº de CPUs).
/// Onde: testes de fixture (CachyOS, sem cgroup v2, hostil).
#[derive(Debug, Default, Clone)]
pub struct FonteFalsa {
    /// caminho -> conteúdo.
    pub arquivos: HashMap<String, String>,
    /// "prog arg1 arg2" -> stdout.
    pub comandos: HashMap<String, String>,
    /// CPUs devolvidas por `cpus()`.
    pub n_cpus: Option<usize>,
}

impl FonteFalsa {
    /// O quê: construtor vazio. Onde: início de toda fixture.
    pub fn nova() -> Self {
        Self::default()
    }
    /// O quê: adiciona um arquivo (encadeável). Onde: montagem de fixtures.
    pub fn arquivo(mut self, caminho: &str, conteudo: &str) -> Self {
        self.arquivos.insert(caminho.to_string(), conteudo.to_string());
        self
    }
    /// O quê: adiciona a saída de um comando (chave `prog args...`). Onde: fixtures.
    pub fn comando(mut self, linha: &str, saida: &str) -> Self {
        self.comandos.insert(linha.to_string(), saida.to_string());
        self
    }
    /// O quê: define as CPUs. Onde: fixtures.
    pub fn cpus_n(mut self, n: usize) -> Self {
        self.n_cpus = Some(n);
        self
    }
}

impl Fonte for FonteFalsa {
    /// O quê: busca no mapa; ausente vira `Ausente`. Onde: ver trait.
    fn ler_arquivo(&self, caminho: &str) -> Result<String, ErroSistema> {
        self.arquivos.get(caminho).cloned().ok_or_else(|| ErroSistema::Ausente(caminho.to_string()))
    }

    /// O quê: deduz o diretório pelos prefixos das chaves. Onde: ver trait.
    fn listar_dir(&self, caminho: &str) -> Result<Vec<String>, ErroSistema> {
        let pref = format!("{}/", caminho.trim_end_matches('/'));
        let mut nomes: Vec<String> = self
            .arquivos
            .keys()
            .filter_map(|k| k.strip_prefix(&pref))
            .filter_map(|r| r.split('/').next())
            .map(str::to_string)
            .collect();
        nomes.sort();
        nomes.dedup();
        if nomes.is_empty() {
            Err(ErroSistema::Ausente(caminho.to_string()))
        } else {
            Ok(nomes)
        }
    }

    /// O quê: busca `prog args` no mapa. Onde: ver trait.
    fn rodar_cmd(&self, prog: &str, args: &[&str], _t: Duration) -> Result<String, ErroSistema> {
        let chave = std::iter::once(prog).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
        self.comandos.get(&chave).cloned().ok_or(ErroSistema::Ausente(chave))
    }

    /// O quê: devolve `n_cpus`. Onde: ver trait.
    fn cpus(&self) -> Option<usize> {
        self.n_cpus
    }
}
