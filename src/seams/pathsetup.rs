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
//! ACONSELHA: nunca escreve nada, e devolve DADOS ([`Orientacao`]), não prosa — o texto
//! (i18n) é do app hospedeiro.
//!
//! **Por que existe (anti-padrão 48):** num CachyOS recém-instalado (rustup do pacman, shell
//! fish) a pasta nunca entrou no PATH e "reabra o terminal" não resolvia nada.
//!
//! Windows: [`PathSetupUnix`] não existe lá; as funções puras compilam em qualquer alvo.

use std::path::{Path, PathBuf};

/// Os `rc` que recebem o export, na ordem original.
const RCS: [&str; 3] = [".bashrc", ".profile", ".zshrc"];

/// **O quê:** o shell do usuário, reduzido ao que muda o comando de PATH.
/// **Onde:** [`Orientacao::shell`], derivado de `$SHELL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TipoShell {
    /// bash.
    Bash,
    /// zsh.
    Zsh,
    /// fish.
    Fish,
    /// Qualquer outro (ou `SHELL` vazio/desconhecido).
    Outro,
}

/// **O quê:** a orientação estruturada de como pôr uma pasta no PATH (sem texto traduzível).
/// **Onde:** devolvida por [`PathSetup::orientacao`]; o host monta a mensagem no seu idioma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orientacao {
    /// Shell detectado.
    pub shell: TipoShell,
    /// `rc` a editar (absoluto sob o home); `None` para [`TipoShell::Outro`].
    pub arquivo_rc: Option<PathBuf>,
    /// A linha de comando exata a rodar (`fish_add_path ...` ou `echo 'export ...' >> rc`).
    pub linha: String,
}

/// **O quê:** a costura de "pôr uma pasta no PATH do usuário".
///
/// **Onde:** os apps recebem um `Arc<dyn PathSetup>` na composição; testes injetam um falso.
pub trait PathSetup: Send + Sync {
    /// **O quê:** garante que `dir` esteja no PATH do usuário (idempotente).
    /// **Onde:** fim de `install`/`update`. Erro é `String` acionável.
    fn garantir_no_path(&self, dir: &Path) -> Result<(), String>;

    /// **O quê:** como pôr `dir` no PATH; `None` se já está no PATH (ou se não há orientação
    /// segura: HOME ausente ou `dir` com caracter de controle/não-UTF-8 — `garantir_no_path`
    /// reporta esses casos como erro). Nunca edita nada.
    /// **Onde:** fim de `install`/`update`; o host formata a mensagem (i18n).
    fn orientacao(&self, dir: &Path) -> Option<Orientacao>;
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

    /// **O quê:** lê PATH, HOME e SHELL do ambiente e delega a [`orientacao_em`]; sem HOME,
    /// `None`. **Onde:** fim de `install`/`update`.
    fn orientacao(&self, dir: &Path) -> Option<Orientacao> {
        let path = std::env::var("PATH").unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        let home = crate::home::home_do_ambiente()?;
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

/// **O quê:** núcleo puro da orientação: `None` se `dir` já está em `path` (ou não há linha
/// segura, ver [`PathSetup::orientacao`]); senão o [`Orientacao`] do shell `shell` (valor de
/// `$SHELL`). Nunca edita nada. **Onde:** [`PathSetupUnix`] e os testes.
pub fn orientacao_em(path: &str, dir: &Path, home: &Path, shell: &str) -> Option<Orientacao> {
    if esta_no_path(path, dir, home) {
        return None;
    }
    let alvo = alvo_de(dir, home).ok()?;
    let nome = shell.rsplit('/').next().unwrap_or("");
    let (tipo, rc, linha) = match nome {
        "fish" => (
            TipoShell::Fish,
            Some(home.join(".config/fish/config.fish")),
            format!("fish_add_path {}", alvo.fish),
        ),
        "bash" => (TipoShell::Bash, Some(home.join(".bashrc")), linha_export(&alvo, "~/.bashrc")),
        "zsh" => (TipoShell::Zsh, Some(home.join(".zshrc")), linha_export(&alvo, "~/.zshrc")),
        _ => (TipoShell::Outro, None, linha_export(&alvo, "~/.profile")),
    };
    Some(Orientacao { shell: tipo, arquivo_rc: rc, linha })
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

    /// **O quê:** atalho: orientação com PATH sem a pasta. **Onde:** testes.
    fn ori(d: &Path, shell: &str) -> Option<Orientacao> {
        orientacao_em(SEM, d, &home(), shell)
    }

    /// **O quê:** bash: campos e `linha` byte-idêntica. **Onde:** snapshot da struct.
    #[test]
    fn orientacao_bash() {
        let o = ori(&dir(), "/bin/bash").expect("orienta");
        assert_eq!(o.shell, TipoShell::Bash);
        assert_eq!(o.arquivo_rc, Some(PathBuf::from("/home/ana/.bashrc")));
        assert_eq!(o.linha, "echo 'export PATH=\"$HOME/.cargo/bin:$PATH\"' >> ~/.bashrc");
    }

    /// **O quê:** zsh com SHELL em caminho. **Onde:** snapshot da struct.
    #[test]
    fn orientacao_zsh() {
        let o = ori(&dir(), "/usr/bin/zsh").expect("orienta");
        assert_eq!(o.shell, TipoShell::Zsh);
        assert_eq!(o.arquivo_rc, Some(PathBuf::from("/home/ana/.zshrc")));
        assert_eq!(o.linha, "echo 'export PATH=\"$HOME/.cargo/bin:$PATH\"' >> ~/.zshrc");
    }

    /// **O quê:** fish com SHELL em caminho. **Onde:** snapshot da struct.
    #[test]
    fn orientacao_fish() {
        let o = ori(&dir(), "/usr/bin/fish").expect("orienta");
        assert_eq!(o.shell, TipoShell::Fish);
        assert_eq!(o.arquivo_rc, Some(PathBuf::from("/home/ana/.config/fish/config.fish")));
        assert_eq!(o.linha, "fish_add_path ~/.cargo/bin");
    }

    /// **O quê:** shell desconhecido/vazio/hostil cai em `Outro` (linha do `.profile`, sem rc).
    /// **Onde:** borda de `$SHELL`.
    #[test]
    fn orientacao_outro_e_vazio() {
        for s in ["", "/usr/bin/nu", "bash\n", "/", "fish ", "FISH", "/bin/zsh/", "\0", "ção"] {
            let o = ori(&dir(), s).expect("orienta");
            assert_eq!(o.shell, TipoShell::Outro, "{s:?}");
            assert_eq!(o.arquivo_rc, None, "{s:?}");
            assert_eq!(
                o.linha, "echo 'export PATH=\"$HOME/.cargo/bin:$PATH\"' >> ~/.profile",
                "{s:?}"
            );
        }
    }

    /// **O quê:** SHELL com caminhos variados mapeia ao tipo certo. **Onde:** borda de `$SHELL`.
    #[test]
    fn shell_com_caminho() {
        for (s, t) in [
            ("/usr/bin/fish", TipoShell::Fish),
            ("/bin/zsh", TipoShell::Zsh),
            ("/usr/local/bin/bash", TipoShell::Bash),
            ("fish", TipoShell::Fish),
        ] {
            assert_eq!(ori(&dir(), s).expect("orienta").shell, t, "{s}");
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
            assert_eq!(orientacao_em(p, &dir(), &home(), "/usr/bin/fish"), None, "{p}");
        }
    }

    /// **O quê:** pasta parecida, vazia ou PATH vazio não contam. **Onde:** `esta_no_path`.
    #[test]
    fn dir_parecido_nao_conta() {
        for p in ["/home/ana/.cargo/bin-velho:/home/ana/.cargo", "", "::", "  "] {
            assert!(orientacao_em(p, &dir(), &home(), "/bin/bash").is_some(), "{p:?}");
        }
    }

    /// **O quê:** `dir` com barra final é comparado normalizado. **Onde:** `esta_no_path`.
    #[test]
    fn dir_com_barra_final() {
        let d = PathBuf::from("/home/ana/.cargo/bin/");
        assert_eq!(orientacao_em("/home/ana/.cargo/bin", &d, &home(), "/bin/bash"), None);
    }

    /// **O quê:** dirs hostis (espaço, aspas, `$`, unicode) geram comando bem citado.
    /// **Onde:** `alvo_de`/`linha_export`.
    #[test]
    fn dir_hostil_e_citado() {
        let d = PathBuf::from("/home/ana/meu app/it's \"x\" $y ção/bin");
        let m = ori(&d, "/bin/bash").expect("orienta");
        assert!(
            m.linha.contains(r#"export PATH="$HOME/meu app/it'\''s \"x\" \$y ção/bin:$PATH""#),
            "{}",
            m.linha
        );
        let f = ori(&d, "/usr/bin/fish").expect("orienta");
        assert!(f.linha.starts_with(r"fish_add_path '/home/ana/meu app/it\'s"), "{}", f.linha);
    }

    /// **O quê:** dir só com espaços/unicode (seguro ou não) e fora do home. **Onde:** `alvo_de`.
    #[test]
    fn dir_unicode_e_fora_do_home() {
        let u = ori(Path::new("/home/ana/ção"), "/usr/bin/fish").expect("orienta");
        assert_eq!(u.linha, "fish_add_path ~/ção");
        let e = ori(Path::new("/opt/meu app/bin"), "/usr/bin/fish").expect("orienta");
        assert_eq!(e.linha, "fish_add_path '/opt/meu app/bin'");
        let b = ori(Path::new("/opt/meu app/bin"), "/bin/zsh").expect("orienta");
        assert_eq!(b.linha, "echo 'export PATH=\"/opt/meu app/bin:$PATH\"' >> ~/.zshrc");
    }

    /// **O quê:** newline/NUL no caminho: sem orientação (nunca comando quebrado) e
    /// `garantir_no_path_em` devolve erro. **Onde:** `alvo_de`.
    #[test]
    fn dir_com_newline_e_recusado() {
        for s in ["/home/ana/a\nb", "/home/ana/a\0b"] {
            let d = PathBuf::from(s);
            assert_eq!(ori(&d, "/bin/bash"), None, "{s:?}");
            let e = garantir_no_path_em(&d, &tmp_home("nl")).expect_err("deve recusar");
            assert!(e.contains("controle"), "{e}");
        }
    }

    /// **O quê:** dir vazio é aceito sem pânico (cai como absoluto vazio). **Onde:** borda.
    #[test]
    fn dir_vazio_nao_entra_em_panico() {
        assert!(ori(Path::new(""), "/bin/bash").is_some());
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
