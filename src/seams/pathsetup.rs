//! Seam de PATH — põe a pasta de instalação no PATH do usuário, ou diz como fazê-lo.
//!
//! **O quê:** o trait [`PathSetup`] (a costura) e sua implementação Unix [`PathSetupUnix`].
//! Reúne duas lógicas que viviam separadas no `schematize_market_rs`: o acréscimo idempotente
//! do `export PATH` nos `rc` de shell (`nucleo/plataforma.rs`: `LINHA_PATH`,
//! `acrescenta_path_no_rc`, `ensure_path_setup`) e a orientação sem culpa
//! (`nucleo/dica_path.rs`: `esta_no_path`, `orientar_path`, `linha_export`,
//! `aviso_pos_instalacao`).
//!
//! **Onde:** o fim de toda instalação/atualização de um app da casa.
//!
//! **Comportamento (herdado):** `garantir_no_path` EDITA os `rc` (`.bashrc`, `.profile`,
//! `.zshrc`), sempre em modo `append` e idempotente (não repete a linha). `orientacao` só
//! ACONSELHA: nunca escreve nada. O texto de `orientacao` é o do locale `en` do market (a lib
//! é std-only e não traz i18n); o app que quiser traduzir usa as peças puras.
//!
//! **Por que existe (anti-padrão 48):** num CachyOS recém-instalado (rustup do pacman, shell
//! fish) a pasta nunca entrou no PATH e "reabra o terminal" não resolvia nada.
//!
//! Windows: [`PathSetupUnix`] não existe lá; as funções puras compilam em qualquer alvo.

use std::path::Path;

/// Texto da dica (locale `en` do market, chave `path.hint`).
const DICA: &str = "The folder {dir} is not in your PATH yet — that is why the command may not be found, and reopening the terminal alone will not fix it.\nTo fix it, run this line once{saved}:\n    {command}";
/// Complemento "salvo em" (locale `en`, chave `path.saved_in`).
const SALVO_EM: &str = "(saved in {file})";
/// Os `rc` que recebem o export, na ordem original.
const RCS: [&str; 3] = [".bashrc", ".profile", ".zshrc"];

/// **O quê:** a costura de "pôr uma pasta no PATH do usuário".
///
/// **Onde:** os apps recebem um `Arc<dyn PathSetup>` na composição; testes injetam um falso.
pub trait PathSetup: Send + Sync {
    /// **O quê:** garante que `dir` esteja no PATH do usuário (idempotente).
    /// **Onde:** fim de `install`/`update`. Erro é `String` acionável.
    fn garantir_no_path(&self, dir: &Path) -> Result<(), String>;

    /// **O quê:** a orientação (texto) de como pôr `dir` no PATH; `""` se já está no PATH.
    /// Nunca edita nada. **Onde:** fim de `install`/`update`, que imprimem o resultado.
    fn orientacao(&self, dir: &Path) -> String;
}

/// **O quê:** implementação Unix: edita os `rc` de shell e lê `PATH`/`SHELL` do ambiente.
///
/// **Onde:** composição dos apps em Linux/macOS/BSD.
#[cfg(unix)]
#[derive(Debug, Default, Clone, Copy)]
pub struct PathSetupUnix;

#[cfg(unix)]
impl PathSetup for PathSetupUnix {
    /// **O quê:** resolve o HOME do ambiente e delega a [`garantir_no_path_em`].
    /// **Onde:** fim de toda instalação.
    fn garantir_no_path(&self, dir: &Path) -> Result<(), String> {
        let home = crate::home::home_do_ambiente().ok_or_else(|| {
            "HOME não definido (ou relativo); não sei onde ficam os rc".to_string()
        })?;
        garantir_no_path_em(dir, &home)
    }

    /// **O quê:** lê PATH, HOME e SHELL do ambiente e delega a [`orientacao_em`].
    /// **Onde:** fim de `install`/`update`.
    fn orientacao(&self, dir: &Path) -> String {
        let path = std::env::var("PATH").unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        let Some(home) = crate::home::home_do_ambiente() else {
            return "HOME não definido; defina HOME e rode de novo.".to_string();
        };
        orientacao_em(&path, dir, &home, &shell)
    }
}

/// **O quê:** normaliza uma entrada de PATH para comparar: expande `~` e `$HOME`, tira barras
/// finais. **Onde:** [`esta_no_path`].
fn normalizar(entrada: &str, home: &str) -> String {
    let mut e = entrada.trim().to_string();
    for pre in ["~", "$HOME", "${HOME}"] {
        if e == pre || e.starts_with(&format!("{pre}/")) {
            e = format!("{home}{}", &e[pre.len()..]);
            break;
        }
    }
    while e.len() > 1 && e.ends_with('/') {
        e.pop();
    }
    e
}

/// **O quê:** `true` se `dir` aparece em `path` (lista separada por `:`), ignorando barra final
/// e a grafia `~`/`$HOME`/absoluto. **Onde:** [`orientacao_em`].
pub fn esta_no_path(path: &str, dir: &Path, home: &Path) -> bool {
    let home = home.to_string_lossy();
    let alvo = normalizar(&dir.to_string_lossy(), &home);
    path.split(':').filter(|p| !p.trim().is_empty()).any(|p| normalizar(p, &home) == alvo)
}

/// **O quê:** `dir` relativo ao `home` (`.cargo/bin`), ou `None` se está fora do home (ou é o
/// próprio home). **Onde:** [`alvo_de`].
fn rel_sob_home(dir: &Path, home: &Path) -> Option<String> {
    let rel = dir.strip_prefix(home).ok()?.to_str()?;
    (!rel.is_empty()).then(|| rel.to_string())
}

/// **O quê:** escapa `\`, `"`, `$` e crase para ir dentro de aspas duplas do shell.
/// **Onde:** [`alvo_de`].
fn escapa_dq(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '"' | '$' | '`') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// Como a pasta aparece nos comandos gerados.
struct Alvo {
    /// Rótulo p/ comentário e "contém": `~/.cargo/bin` ou o absoluto.
    mostra: String,
    /// Trecho que, se já está no `rc`, evita duplicar (`.cargo/bin` ou o absoluto).
    marca: String,
    /// Valor dentro das aspas do export: `$HOME/.cargo/bin` ou o absoluto escapado.
    valor: String,
    /// Argumento do `fish_add_path`: `~/.cargo/bin` ou o absoluto entre aspas simples.
    fish: String,
}

/// **O quê:** monta o [`Alvo`] de `dir`. `Err` p/ caracter de controle (newline/NUL quebraria
/// a linha do `rc`) ou caminho não-UTF-8. **Onde:** [`linha_rc`], [`orientacao_em`].
fn alvo_de(dir: &Path, home: &Path) -> Result<Alvo, String> {
    let abs = dir.to_str().ok_or_else(|| format!("{}: caminho não é UTF-8", dir.display()))?;
    if abs.chars().any(char::is_control) {
        return Err(format!("{abs:?}: caminho com caracter de controle; não dá pra pôr no PATH"));
    }
    let fish_q = |s: &str| format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"));
    Ok(match rel_sob_home(dir, home) {
        Some(rel) => {
            let seguro = rel.chars().all(|c| c.is_alphanumeric() || "._-/+@".contains(c));
            let fish = if seguro { format!("~/{rel}") } else { fish_q(abs) };
            Alvo {
                mostra: format!("~/{rel}"),
                valor: format!("$HOME/{}", escapa_dq(&rel)),
                marca: rel,
                fish,
            }
        }
        None => Alvo {
            mostra: abs.to_string(),
            marca: abs.to_string(),
            valor: escapa_dq(abs),
            fish: fish_q(abs),
        },
    })
}

/// **O quê:** o bloco que vai no `rc` (para `~/.cargo/bin`, idêntico ao `LINHA_PATH` do market).
/// **Onde:** [`acrescenta_path_no_rc`].
fn linha_rc(a: &Alvo) -> String {
    format!("\n# schematize-market: {} no PATH\nexport PATH=\"{}:$PATH\"\n", a.mostra, a.valor)
}

/// **O quê:** o comando que acrescenta o export ao arquivo `rc` dado (era `linha_export`).
/// **Onde:** [`orientacao_em`], para bash, zsh e o fallback `~/.profile`.
fn linha_export(a: &Alvo, rc: &str) -> String {
    let export = format!("export PATH=\"{}:$PATH\"", a.valor).replace('\'', "'\\''");
    format!("echo '{export}' >> {rc}")
}

/// **O quê:** acrescenta `linha` a UM `rc`, sem nunca reescrevê-lo por inteiro. `Ok(true)` se
/// acrescentou, `Ok(false)` se `marca` já estava lá, `Err` se o arquivo existe mas não pôde ser
/// lido — e então **não escreve nada**.
///
/// **Onde:** [`garantir_no_path_em`], uma vez por `rc`.
///
/// **Por que `append`:** ler-modificar-escrever transformava toda falha de leitura (p.ex. um
/// `.bashrc` Latin-1, `InvalidData`) em "arquivo vazio" e reescrevia o rc só com o export. Em
/// `append` não há como truncar: o pior caso é uma linha duplicada.
pub fn acrescenta_path_no_rc(p: &Path, marca: &str, linha: &str) -> Result<bool, String> {
    match std::fs::read_to_string(p) {
        Ok(cur) if cur.contains(marca) => return Ok(false),
        Ok(_) => {}
        // Não existir é normal: o `create(true)` do append cria.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!("{}: não deu pra ler ({e}); deixei o arquivo intacto", p.display()))
        }
    }
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(p)
        .and_then(|mut f| std::io::Write::write_all(&mut f, linha.as_bytes()))
        .map(|_| true)
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// **O quê:** núcleo injetável de `garantir_no_path`: acrescenta o export de `dir` aos três
/// `rc` sob `home`. Tenta todos; os erros (nunca engolidos) voltam juntos, com a linha para
/// colar à mão. **Onde:** [`PathSetupUnix`] e os testes.
pub fn garantir_no_path_em(dir: &Path, home: &Path) -> Result<(), String> {
    let alvo = alvo_de(dir, home)?;
    let linha = linha_rc(&alvo);
    let erros: Vec<String> = RCS
        .iter()
        .filter_map(|rc| acrescenta_path_no_rc(&home.join(rc), &alvo.marca, &linha).err())
        .collect();
    if erros.is_empty() {
        return Ok(());
    }
    Err(format!("{}. Acrescente manualmente:{linha}", erros.join("; ")))
}

/// **O quê:** núcleo puro da orientação: `""` se `dir` já está em `path`; senão a dica com a
/// ÚNICA linha que resolve no shell `shell` (o valor de `$SHELL`). Nunca edita nada.
/// **Onde:** [`PathSetupUnix`] e os testes.
pub fn orientacao_em(path: &str, dir: &Path, home: &Path, shell: &str) -> String {
    if esta_no_path(path, dir, home) {
        return String::new();
    }
    let alvo = match alvo_de(dir, home) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let nome = shell.rsplit('/').next().unwrap_or("");
    let (arquivo, comando) = match nome {
        "fish" => (None, format!("fish_add_path {}", alvo.fish)),
        "bash" => (Some("~/.bashrc"), linha_export(&alvo, "~/.bashrc")),
        "zsh" => (Some("~/.zshrc"), linha_export(&alvo, "~/.zshrc")),
        _ => (Some("~/.profile"), linha_export(&alvo, "~/.profile")),
    };
    let salvo = arquivo.map(|a| format!(" {}", SALVO_EM.replace("{file}", a))).unwrap_or_default();
    DICA.replace("{dir}", &dir.display().to_string())
        .replace("{saved}", &salvo)
        .replace("{command}", &comando)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SEM: &str = "/usr/bin:/bin";

    /// **O quê:** HOME falso fixo. **Onde:** testes puros.
    fn home() -> PathBuf {
        PathBuf::from("/home/ana")
    }

    /// **O quê:** a pasta cargo do HOME falso. **Onde:** testes puros.
    fn dir() -> PathBuf {
        PathBuf::from("/home/ana/.cargo/bin")
    }

    /// **O quê:** cria um HOME temporário único sob `target/testes` (nunca o HOME real).
    /// **Onde:** testes que escrevem em disco.
    fn tmp_home(rotulo: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/testes").join(format!(
            "pathsetup-{rotulo}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).expect("criar home temporário");
        p
    }

    /// **O quê:** lê um rc ou "" se não existe. **Onde:** asserts dos testes de disco.
    fn le(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    /// **O quê:** snapshot byte a byte da dica do bash. **Onde:** garante texto idêntico ao market.
    #[test]
    fn snapshot_bash() {
        let m = orientacao_em(SEM, &dir(), &home(), "/bin/bash");
        assert_eq!(m, "The folder /home/ana/.cargo/bin is not in your PATH yet — that is why the command may not be found, and reopening the terminal alone will not fix it.\nTo fix it, run this line once (saved in ~/.bashrc):\n    echo 'export PATH=\"$HOME/.cargo/bin:$PATH\"' >> ~/.bashrc");
    }

    /// **O quê:** snapshot da dica do zsh. **Onde:** texto idêntico ao market.
    #[test]
    fn snapshot_zsh() {
        let m = orientacao_em(SEM, &dir(), &home(), "/usr/bin/zsh");
        assert!(m.ends_with("run this line once (saved in ~/.zshrc):\n    echo 'export PATH=\"$HOME/.cargo/bin:$PATH\"' >> ~/.zshrc"), "{m}");
    }

    /// **O quê:** snapshot da dica do fish (sem "saved in"). **Onde:** texto idêntico ao market.
    #[test]
    fn snapshot_fish() {
        let m = orientacao_em(SEM, &dir(), &home(), "/usr/bin/fish");
        assert_eq!(m, "The folder /home/ana/.cargo/bin is not in your PATH yet — that is why the command may not be found, and reopening the terminal alone will not fix it.\nTo fix it, run this line once:\n    fish_add_path ~/.cargo/bin");
        assert!(!m.contains("reabra"));
    }

    /// **O quê:** snapshot do shell desconhecido/vazio (cai no `.profile`). **Onde:** idem.
    #[test]
    fn snapshot_outro_e_vazio() {
        for s in ["", "/usr/bin/nu", "bash\n", "/"] {
            let m = orientacao_em(SEM, &dir(), &home(), s);
            assert!(m.ends_with("once (saved in ~/.profile):\n    echo 'export PATH=\"$HOME/.cargo/bin:$PATH\"' >> ~/.profile"), "{s:?}: {m}");
        }
    }

    /// **O quê:** o bloco do rc é byte-idêntico ao `LINHA_PATH` do market. **Onde:** snapshot.
    #[test]
    fn snapshot_linha_rc() {
        let a = alvo_de(&dir(), &home()).expect("alvo");
        assert_eq!(
            linha_rc(&a),
            "\n# schematize-market: ~/.cargo/bin no PATH\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n"
        );
    }

    /// **O quê:** já no PATH, em todas as grafias, a orientação é vazia. **Onde:** `esta_no_path`.
    #[test]
    fn ja_no_path_nao_diz_nada() {
        for p in [
            "/usr/bin:/home/ana/.cargo/bin",
            "/home/ana/.cargo/bin/:/usr/bin",
            "~/.cargo/bin:/usr/bin",
            "/usr/bin:$HOME/.cargo/bin/",
            "/usr/bin:${HOME}/.cargo/bin",
        ] {
            assert_eq!(orientacao_em(p, &dir(), &home(), "/usr/bin/fish"), "", "{p}");
        }
    }

    /// **O quê:** pasta parecida, vazia ou PATH vazio não contam. **Onde:** `esta_no_path`.
    #[test]
    fn dir_parecido_nao_conta() {
        for p in ["/home/ana/.cargo/bin-velho:/home/ana/.cargo", "", "::", "  "] {
            assert!(!orientacao_em(p, &dir(), &home(), "/bin/bash").is_empty(), "{p:?}");
        }
    }

    /// **O quê:** `dir` com barra final é comparado normalizado. **Onde:** `esta_no_path`.
    #[test]
    fn dir_com_barra_final() {
        let d = PathBuf::from("/home/ana/.cargo/bin/");
        assert_eq!(orientacao_em("/home/ana/.cargo/bin", &d, &home(), "/bin/bash"), "");
    }

    /// **O quê:** dirs hostis (espaço, aspas, `$`, unicode) geram comando bem citado.
    /// **Onde:** `alvo_de`/`linha_export`.
    #[test]
    fn dir_hostil_e_citado() {
        let d = PathBuf::from("/home/ana/meu app/it's \"x\" $y ção/bin");
        let m = orientacao_em(SEM, &d, &home(), "/bin/bash");
        assert!(
            m.contains(r#"export PATH="$HOME/meu app/it'\''s \"x\" \$y ção/bin:$PATH""#),
            "{m}"
        );
        let f = orientacao_em(SEM, &d, &home(), "/usr/bin/fish");
        assert!(f.contains(r"fish_add_path '/home/ana/meu app/it\'s"), "{f}");
    }

    /// **O quê:** newline/NUL no caminho vira erro explicativo, nunca comando quebrado.
    /// **Onde:** `alvo_de`.
    #[test]
    fn dir_com_newline_e_recusado() {
        for s in ["/home/ana/a\nb", "/home/ana/a\0b"] {
            let d = PathBuf::from(s);
            let m = orientacao_em(SEM, &d, &home(), "/bin/bash");
            assert!(m.contains("controle") && !m.contains("echo"), "{m}");
            let e = garantir_no_path_em(&d, &tmp_home("nl")).expect_err("deve recusar");
            assert!(e.contains("controle"), "{e}");
        }
    }

    /// **O quê:** dir vazio é aceito sem pânico (cai como absoluto vazio). **Onde:** borda.
    #[test]
    fn dir_vazio_nao_entra_em_panico() {
        let m = orientacao_em(SEM, Path::new(""), &home(), "/bin/bash");
        assert!(m.starts_with("The folder  is not"), "{m}");
    }

    /// **O quê:** idempotência: duas chamadas = uma linha por rc; cria os 3 rc. **Onde:** disco.
    #[test]
    fn garantir_e_idempotente() {
        let h = tmp_home("idem");
        let d = h.join(".cargo/bin");
        garantir_no_path_em(&d, &h).expect("1ª");
        garantir_no_path_em(&d, &h).expect("2ª");
        for rc in RCS {
            let c = le(&h.join(rc));
            assert_eq!(c.matches("export PATH").count(), 1, "{rc}: {c}");
            assert_eq!(c, "\n# schematize-market: ~/.cargo/bin no PATH\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n");
        }
    }

    /// **O quê:** conteúdo prévio é preservado (append, nunca reescreve). **Onde:** disco.
    #[test]
    fn preserva_conteudo_existente() {
        let h = tmp_home("preserva");
        std::fs::write(h.join(".bashrc"), "alias x='y'\n# acentuação\n").expect("seed");
        garantir_no_path_em(&h.join(".cargo/bin"), &h).expect("ok");
        let c = le(&h.join(".bashrc"));
        assert!(c.starts_with("alias x='y'\n# acentuação\n") && c.contains("export PATH"), "{c}");
    }

    /// **O quê:** rc ilegível (Latin-1) fica intacto e o erro volta com a linha manual.
    /// **Onde:** disco; protege contra o bug histórico de truncar o `.bashrc`.
    #[test]
    fn rc_nao_utf8_fica_intacto_e_erra() {
        let h = tmp_home("latin1");
        let bruto = b"# coment\xe1rio\n".to_vec();
        std::fs::write(h.join(".bashrc"), &bruto).expect("seed");
        let e = garantir_no_path_em(&h.join(".cargo/bin"), &h).expect_err("deve avisar");
        assert!(e.contains(".bashrc") && e.contains("intacto") && e.contains("Acrescente"), "{e}");
        assert_eq!(std::fs::read(h.join(".bashrc")).expect("ler"), bruto);
        assert_eq!(le(&h.join(".profile")).matches("export PATH").count(), 1, "outros rc seguem");
    }

    /// **O quê:** HOME com espaço, aspas, `$` e unicode funciona. **Onde:** disco.
    #[test]
    fn home_hostil_funciona() {
        let h = tmp_home("meu home 'q' $x ç");
        let d = h.join("bin local");
        garantir_no_path_em(&d, &h).expect("ok");
        let c = le(&h.join(".zshrc"));
        assert!(c.contains("export PATH=\"$HOME/bin local:$PATH\""), "{c}");
        garantir_no_path_em(&d, &h).expect("de novo");
        assert_eq!(le(&h.join(".zshrc")), c);
    }

    /// **O quê:** dir fora do HOME entra como absoluto escapado e também é idempotente.
    /// **Onde:** disco.
    #[test]
    fn dir_fora_do_home_usa_absoluto() {
        let h = tmp_home("fora");
        let d = PathBuf::from("/opt/app \"x\"/bin");
        garantir_no_path_em(&d, &h).expect("ok");
        garantir_no_path_em(&d, &h).expect("ok2");
        let c = le(&h.join(".profile"));
        assert_eq!(c.matches("export PATH").count(), 1);
        assert!(c.contains(r#"export PATH="/opt/app \"x\"/bin:$PATH""#), "{c}");
    }

    /// **O quê:** HOME inexistente: erro de I/O reportado, nunca engolido. **Onde:** disco.
    #[test]
    fn home_inexistente_devolve_erro() {
        let h = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/testes/nao-existe-xyz");
        let e = garantir_no_path_em(&h.join(".cargo/bin"), &h).expect_err("sem home");
        assert!(e.contains(".bashrc") && e.contains("Acrescente"), "{e}");
    }

    /// **O quê:** o trait é objeto-seguro e `Send + Sync`. **Onde:** composição dos apps.
    #[cfg(unix)]
    #[test]
    fn trait_e_objeto_seguro() {
        let _p: std::sync::Arc<dyn PathSetup> = std::sync::Arc::new(PathSetupUnix);
    }
}
