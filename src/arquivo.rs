//! Escrita de arquivo sem deixar nada pela metade.
//!
//! **O quê:** [`escreve_atomico`] — temporário único + `sync_all` + `rename`.
//!
//! **Onde:** qualquer app que grave estado (`state.json`, config) que não pode ser lido
//! truncado por outro processo/thread nem sobreviver corrompido a um corte de energia.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// **O quê:** grava `conteudo` em `alvo` sem deixar arquivo pela metade.
///
/// **Onde:** estado e configuração dos apps. Cria os diretórios pai. Erros vêm como `String`
/// legível com o caminho (contrato herdado das 4 cópias que esta função substitui).
///
/// ## O temporário é ÚNICO POR CHAMADA, e isso foi pago
///
/// Antes o nome era `.{alvo}.tmp-{pid}`: duas THREADS gravando o mesmo alvo usavam o MESMO
/// temporário. A segunda truncava o arquivo da primeira, a primeira renomeava, e a segunda
/// falhava com "No such file or directory" — em ~15% das rodadas. O contador resolve entre
/// threads; o PID, entre processos. Se algo falha, o temporário é removido.
pub fn escreve_atomico(alvo: &Path, conteudo: &str) -> Result<(), String> {
    let dir = dir_do_alvo(alvo);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{}.tmp-{}-{n}",
        alvo.file_name().and_then(|s| s.to_str()).unwrap_or("alvo"),
        std::process::id()
    ));
    let r = grava_e_sincroniza(&tmp, conteudo)
        .and_then(|()| std::fs::rename(&tmp, alvo).map_err(|e| format!("{}: {e}", alvo.display())));
    if let Err(causa) = r {
        return match std::fs::remove_file(&tmp) {
            Ok(()) => Err(causa),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(causa),
            Err(e) => Err(format!("{causa} (e não removi {}: {e})", tmp.display())),
        };
    }
    Ok(())
}

/// **O quê:** o diretório onde o temporário nasce: o pai de `alvo`, ou `.` se `alvo` é só um nome.
///
/// **Onde:** [`escreve_atomico`].
fn dir_do_alvo(alvo: &Path) -> &Path {
    alvo.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// **O quê:** cria `tmp`, escreve e faz `sync_all`.
///
/// **Onde:** [`escreve_atomico`]. O `sync_all` vem ANTES do rename: sem ele, um corte de
/// energia entre os dois deixa o nome novo apontando para tamanho certo e conteúdo zerado.
fn grava_e_sincroniza(tmp: &Path, conteudo: &str) -> Result<(), String> {
    let mut f = std::fs::File::create(tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(conteudo.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// **O quê:** pasta temporária única por teste.
    ///
    /// **Onde:** os testes deste módulo.
    fn tmp(nome: &str) -> PathBuf {
        let d = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/target/testes"))
            .join(format!("plat-arq-{}-{nome}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// **O quê:** lista os temporários `.tmp-` que sobraram em `dir`.
    ///
    /// **Onde:** os testes deste módulo.
    fn sobras(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect()
    }

    /// **Escrita atômica não deixa arquivo pela metade, nem entre THREADS.**
    ///
    /// Vinte threads gravando o MESMO alvo: no fim, o conteúdo é um dos vinte inteiro — nunca
    /// mistura, nunca vazio, e nenhuma escrita falha. Com o temporário por PID (sem contador)
    /// este teste falhava em ~15% das rodadas.
    #[test]
    fn escrita_atomica_aguenta_threads_concorrentes() {
        let base = tmp("threads");
        let alvo = base.join("x.json");
        let esperados: Vec<String> = (0..20).map(|i| format!("conteudo-numero-{i:02}")).collect();
        std::thread::scope(|s| {
            for c in &esperados {
                let alvo = alvo.clone();
                s.spawn(move || escreve_atomico(&alvo, c).expect("nenhuma escrita pode falhar"));
            }
        });
        let lido = std::fs::read_to_string(&alvo).expect("o arquivo existe");
        assert!(esperados.contains(&lido), "conteúdo misturado ou parcial: {lido:?}");
        assert!(sobras(&base).is_empty(), "temporário(s) órfão(s)");
    }

    /// Cria pais, sobrescreve, aceita vazio e unicode.
    #[test]
    fn cria_pais_sobrescreve_vazio_e_unicode() {
        let base = tmp("basico");
        let alvo = base.join("a/b/c/estado ç.json");
        escreve_atomico(&alvo, "um").unwrap();
        escreve_atomico(&alvo, "").unwrap();
        assert_eq!(std::fs::read_to_string(&alvo).unwrap(), "");
        escreve_atomico(&alvo, "日本語 \u{0} nulo ✓").unwrap();
        assert_eq!(std::fs::read_to_string(&alvo).unwrap(), "日本語 \u{0} nulo ✓");
        assert!(sobras(alvo.parent().unwrap()).is_empty());
    }

    /// Alvo que é um diretório: erro legível com o caminho, nenhum temporário sobra.
    #[test]
    fn alvo_diretorio_da_erro_e_nao_deixa_temporario() {
        let base = tmp("dir");
        let alvo = base.join("pasta");
        std::fs::create_dir_all(&alvo).unwrap();
        let e = escreve_atomico(&alvo, "x").unwrap_err();
        assert!(e.contains("pasta"), "{e}");
        assert!(sobras(&base).is_empty());
        assert!(alvo.is_dir(), "o diretório segue intocado");
    }

    /// Pai que é arquivo: erro, não pânico.
    #[test]
    fn pai_que_e_arquivo_da_erro() {
        let base = tmp("pai");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("arq"), "x").unwrap();
        assert!(escreve_atomico(&base.join("arq/filho.json"), "x").is_err());
    }

    /// Alvo só com o nome (sem pai) resolve ao cwd "." — decidido por função pura, sem
    /// escrever nada fora do `target`.
    #[test]
    fn alvo_sem_pai_resolve_ao_diretorio_corrente() {
        assert_eq!(dir_do_alvo(Path::new("so-o-nome.json")), Path::new("."));
        assert_eq!(dir_do_alvo(Path::new("a/b.json")), Path::new("a"));
        assert_eq!(dir_do_alvo(Path::new("/b.json")), Path::new("/"));
    }

    /// Alvo com base explícita e nome unicode/espaços grava dentro dela.
    #[test]
    fn alvo_com_base_explicita_grava_dentro_dela() {
        let base = tmp("base");
        let alvo = base.join("só o nome.json");
        escreve_atomico(&alvo, "x").unwrap();
        assert_eq!(std::fs::read_to_string(&alvo).unwrap(), "x");
    }
}
