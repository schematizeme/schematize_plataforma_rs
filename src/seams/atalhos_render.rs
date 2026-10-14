//! Atalhos de menu (freedesktop): a ENTRADA descrita como dado e o RENDER puro do `.desktop`.
//!
//! **O quê:** [`EntradaAtalho`] descreve um atalho de aplicativo (nome, binários, ícone,
//! categorias, classe de janela...) e [`render`] devolve o texto do `.desktop` — função
//! PURA, sem tocar o disco. Unifica o `render` do `schematize-market` e do `schematize-overdev`
//! (`nucleo/desktop.rs`) e o lançador `make_launcher` do hub: o texto sai **byte a byte** igual
//! ao que cada app produzia (travado pelos testes `snapshot_*`).
//!
//! **Onde:** [`super::atalhos::AtalhosUnix`] grava o resultado; os apps montam a
//! [`EntradaAtalho`] e chamam o trait [`super::atalhos::Atalhos`].
//!
//! ## As duas formas do ícone (mantidas dos apps)
//!
//! **Com `gui`:** `Exec=<gui>` e `Terminal=false`. **Sem `gui`:** `Exec=<bin> <args_sem_gui>`
//! e `Terminal=true` — um `.desktop` `Terminal=false` apontando para uma CLI abriria um processo
//! sem janela que morre no mesmo instante (clique que não faz nada, §37.48). Sem `gui` **e**
//! sem `bin` não há o que executar: é erro, não entrada vazia.

use std::path::{Path, PathBuf};

/// Tamanho (px) cujo PNG vira o `Icon=` absoluto do `.desktop`.
pub const TAMANHO_ICONE_PRINCIPAL: u32 = 256;

/// **O quê:** tudo o que varia entre os apps ao montar um atalho de menu.
///
/// **Onde:** montada pelo app (market, overdev, hub) e entregue ao trait
/// [`super::atalhos::Atalhos`]. Os textos são os valores LITERAIS que vão ao `.desktop`
/// (`categorias`/`palavras_chave` recebem os itens e o `;` final é posto pelo render).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntradaAtalho {
    /// HOME do usuário; o `.desktop` e os ícones moram sob ele.
    pub home: PathBuf,
    /// Nome do arquivo `.desktop` SEM extensão (`schematize-market`, `schematize-gui`...).
    pub id: String,
    /// `Name=`.
    pub nome: String,
    /// `GenericName=` (omitido se `None`).
    pub generico: Option<String>,
    /// `Comment=` (omitido se `None`).
    pub comentario: Option<String>,
    /// Executável do CLI (caminho absoluto), usado só SEM janela.
    pub bin: Option<PathBuf>,
    /// Argumentos do CLI no fallback em terminal (`list --wait`, `status`); pode ser vazio.
    pub args_sem_gui: String,
    /// Executável da janela (caminho absoluto). `Some` => `Terminal=false`.
    pub gui: Option<PathBuf>,
    /// Itens de `Categories=`.
    pub categorias: Vec<String>,
    /// Itens de `Keywords=` (omitida se vazio).
    pub palavras_chave: Vec<String>,
    /// `StartupNotify=` (omitido se `None`).
    pub startup_notify: Option<bool>,
    /// `StartupWMClass=` (omitido se `None`): o nome/app_id da JANELA, não o do CLI.
    pub startup_wm_class: Option<String>,
    /// Nome do ícone (sem extensão): `Icon=` cai nele se o PNG principal não existir.
    pub icone_nome: String,
    /// PNGs por tamanho `(px, bytes)`; vão para `hicolor/<px>x<px>/apps/<icone_nome>.png`.
    pub icones: Vec<(u32, Vec<u8>)>,
    /// Copia o PNG de 256px também para pixmaps, `~/.icons` e `icons/` (fallbacks legados).
    pub icone_extras: bool,
    /// `true`: falha ao gravar o ícone é `Err` (market/overdev). `false`: vira aviso (hub).
    pub icone_obrigatorio: bool,
    /// Roda `gtk-update-icon-cache` depois de instalar ícones (hub).
    pub cache_icones: bool,
    /// Outros `.desktop` (ids) nossos e obsoletos a remover ao instalar (hub: `overflow-gui`).
    pub legados: Vec<String>,
}

/// **O quê:** rejeita texto com caracteres de controle (`\n`, `\r`, `\0`...), que injetariam
/// chaves novas no `.desktop`.
///
/// **Onde:** [`validar`], campo a campo.
fn sem_controle(campo: &str, v: &str) -> Result<(), String> {
    if v.chars().any(char::is_control) {
        return Err(format!("{campo} tem caractere de controle (quebra de linha, NUL...)"));
    }
    Ok(())
}

/// **O quê:** valida um identificador usado como nome de arquivo (`id`, `icone_nome`, legado).
///
/// **Onde:** [`validar`]. Vazio, separador de caminho, `..` ou controle escapariam do diretório.
fn ident_valido(campo: &str, v: &str) -> Result<(), String> {
    sem_controle(campo, v)?;
    if v.trim().is_empty() || v.contains('/') || v.contains('\\') || v.contains("..") {
        return Err(format!("{campo} inválido: {v:?}"));
    }
    Ok(())
}

/// **O quê:** valida a entrada inteira (sem tocar o disco).
///
/// **Onde:** [`render`] e `AtalhosUnix::instalar_com_avisos`, ANTES de gravar qualquer coisa.
///
/// **Regras:** `home` absoluto; `nome` não vazio; nenhum texto com controle; itens de
/// categoria/palavra sem `;`; nomes de arquivo sem separador; tem de haver `gui` ou `bin`.
pub fn validar(e: &EntradaAtalho) -> Result<(), String> {
    if !e.home.is_absolute() {
        return Err(format!("home precisa ser absoluto: {}", e.home.display()));
    }
    ident_valido("id", &e.id)?;
    ident_valido("icone_nome", &e.icone_nome)?;
    for l in &e.legados {
        ident_valido("legado", l)?;
    }
    if e.nome.trim().is_empty() {
        return Err("nome vazio".to_string());
    }
    sem_controle("nome", &e.nome)?;
    for (c, v) in [("generico", &e.generico), ("comentario", &e.comentario)] {
        sem_controle(c, v.as_deref().unwrap_or(""))?;
    }
    sem_controle("startup_wm_class", e.startup_wm_class.as_deref().unwrap_or(""))?;
    sem_controle("args_sem_gui", &e.args_sem_gui)?;
    for (c, itens) in [("categorias", &e.categorias), ("palavras_chave", &e.palavras_chave)] {
        for i in itens {
            sem_controle(c, i)?;
            if i.contains(';') {
                return Err(format!("{c}: item com ';' quebraria a lista: {i:?}"));
            }
        }
    }
    if e.gui.is_none() && e.bin.is_none() {
        return Err("sem janela e sem CLI: não há o que o atalho execute".to_string());
    }
    Ok(())
}

/// **O quê:** o caminho como argumento de `Exec=`: idêntico ao `display()` quando seguro; com
/// espaço/tab vai entre aspas; `%` vira `%%` (código de campo da spec). Aspas, crase, `$` e `\`
/// são recusados — a spec exige escapes em camadas que ninguém quer errar.
///
/// **Onde:** [`render`], para `gui` e `bin`. Os apps hoje não citavam (um HOME com espaço
/// quebrava o clique); para caminhos sem espaço a saída é byte a byte a de antes.
fn exec_arg(p: &Path) -> Result<String, String> {
    let s = p.display().to_string();
    sem_controle("caminho do Exec", &s)?;
    if s.chars().any(|c| matches!(c, '"' | '`' | '$' | '\\')) {
        return Err(format!("caminho com caractere proibido em Exec=: {s:?}"));
    }
    let s = s.replace('%', "%%");
    if s.contains(char::is_whitespace) {
        return Ok(format!("\"{s}\""));
    }
    Ok(s)
}

/// **O quê:** o conteúdo do `.desktop`, exatamente como vai para o disco. `icone` é o valor de
/// `Icon=` (caminho absoluto do PNG ou o nome do ícone).
///
/// **Onde:** `AtalhosUnix::instalar_com_avisos` e os testes `snapshot_*`. PURA.
///
/// **Ordem das chaves** (a dos apps): Type, Name, GenericName, Comment, Exec, Icon, Terminal,
/// Categories, Keywords, StartupNotify, StartupWMClass.
pub fn render(e: &EntradaAtalho, icone: &str) -> Result<String, String> {
    validar(e)?;
    sem_controle("icone", icone)?;
    // A janela não recebe argumento; a CLI recebe o que o app definiu (`--wait`, `status`).
    let (exec, terminal) = match (&e.gui, &e.bin) {
        (Some(g), _) => (exec_arg(g)?, "false"),
        (None, Some(b)) => {
            let b = exec_arg(b)?;
            let ex = if e.args_sem_gui.is_empty() { b } else { format!("{b} {}", e.args_sem_gui) };
            (ex, "true")
        }
        (None, None) => return Err("sem janela e sem CLI".to_string()),
    };
    let mut t = format!("[Desktop Entry]\nType=Application\nName={}\n", e.nome);
    if let Some(g) = &e.generico {
        t.push_str(&format!("GenericName={g}\n"));
    }
    if let Some(c) = &e.comentario {
        t.push_str(&format!("Comment={c}\n"));
    }
    t.push_str(&format!("Exec={exec}\nIcon={icone}\nTerminal={terminal}\n"));
    t.push_str(&format!("Categories={}\n", lista(&e.categorias)));
    if !e.palavras_chave.is_empty() {
        t.push_str(&format!("Keywords={}\n", lista(&e.palavras_chave)));
    }
    if let Some(n) = e.startup_notify {
        t.push_str(&format!("StartupNotify={n}\n"));
    }
    if let Some(w) = &e.startup_wm_class {
        t.push_str(&format!("StartupWMClass={w}\n"));
    }
    Ok(t)
}

/// **O quê:** junta os itens no formato freedesktop: `a;b;c;` (com `;` final).
///
/// **Onde:** [`render`] para `Categories=` e `Keywords=`.
fn lista(itens: &[String]) -> String {
    itens.iter().map(|i| format!("{i};")).collect()
}

/// **O quê:** acha a janela IRMÃ do binário (mesmo diretório), devolvendo o caminho absoluto se
/// o arquivo existir. No Windows tenta `<gui>.exe` antes.
///
/// **Onde:** os apps, antes de montar a [`EntradaAtalho`], para preencher `gui`.
///
/// **Por que só a parte pura de `resolver_gui`:** o original caía em `bin::resolve_bin` (PATH e
/// `~/.cargo/bin`), que é acoplado ao ambiente e já mora em `crate::bin`; aqui fica a regra de
/// "o par ao lado ganha do PATH", que é igual nos dois apps e testável sem ambiente.
pub fn resolver_gui_irma(bin_proprio: &Path, gui_bin: &str) -> Option<PathBuf> {
    let exe = format!("{gui_bin}.exe");
    let nomes: Vec<&str> = if cfg!(windows) { vec![&exe, gui_bin] } else { vec![gui_bin] };
    let dir = bin_proprio.parent()?;
    nomes.into_iter().map(|n| dir.join(n)).find(|c| c.is_file())
}

#[cfg(test)]
pub(crate) mod testes_util {
    use super::*;

    /// **O quê:** entrada do `schematize-market` com as strings dos apps (sem janela).
    ///
    /// **Onde:** testes de render e de disco.
    pub fn market(home: &Path) -> EntradaAtalho {
        EntradaAtalho {
            home: home.to_path_buf(),
            id: "schematize-market".into(),
            nome: "schematize Market".into(),
            generico: Some("Instalar e remover programas".into()),
            comentario: Some("Linguagens, ferramentas de dev e os apps do ecossistema".into()),
            bin: Some(PathBuf::from("/h/.cargo/bin/schematize-market")),
            args_sem_gui: "list --wait".into(),
            gui: None,
            categorias: ["Development", "System", "PackageManager"].map(String::from).to_vec(),
            palavras_chave: ["instalar", "install", "programa", "linguagem", "runtime", "pacote"]
                .into_iter()
                .chain(["market"])
                .map(String::from)
                .collect(),
            startup_notify: Some(false),
            startup_wm_class: None,
            icone_nome: "schematize-market".into(),
            icones: vec![(16, b"p16".to_vec()), (256, b"p256".to_vec())],
            icone_extras: true,
            icone_obrigatorio: true,
            cache_icones: false,
            legados: vec![],
        }
    }

    /// **O quê:** entrada do `schematize-overdev` (sem janela).
    ///
    /// **Onde:** testes de render e de disco.
    pub fn overdev(home: &Path) -> EntradaAtalho {
        EntradaAtalho {
            id: "schematize-overdev".into(),
            nome: "schematize Overdev".into(),
            generico: Some("Run, checklist, perguntas".into()),
            comentario: Some(
                "O progresso do run, o checklist e as perguntas que esperam você".into(),
            ),
            bin: Some(PathBuf::from("/h/.cargo/bin/schematize-overdev")),
            args_sem_gui: "status".into(),
            categorias: ["Development", "Utility"].map(String::from).to_vec(),
            palavras_chave: "skill skills catalogo catalog claude agente agent fork"
                .split(' ')
                .map(String::from)
                .collect(),
            startup_wm_class: Some("schematize-overdev-gui".into()),
            icone_nome: "schematize-overdev".into(),
            ..market(home)
        }
    }

    /// **O quê:** entrada do lançador `schematize-gui` do hub (`make_launcher`): só janela.
    ///
    /// **Onde:** testes de render e de disco.
    pub fn hub(home: &Path) -> EntradaAtalho {
        EntradaAtalho {
            id: "schematize-gui".into(),
            nome: "schematize".into(),
            generico: Some("Skills, overdev e mais".into()),
            comentario: Some("Skills, overdev e mais — schematize".into()),
            bin: None,
            args_sem_gui: String::new(),
            gui: Some(PathBuf::from("/h/.cargo/bin/schematize-gui")),
            categorias: ["Development", "Utility"].map(String::from).to_vec(),
            palavras_chave: ["schematize", "skills", "overdev", "claude"]
                .map(String::from)
                .to_vec(),
            startup_notify: None,
            startup_wm_class: Some("schematize-gui".into()),
            icone_nome: "schematize".into(),
            icone_extras: false,
            icone_obrigatorio: false,
            cache_icones: true,
            legados: vec!["overflow-gui".into()],
            ..market(home)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testes_util::*;
    use super::*;

    const ICONE: &str = "/h/.local/share/icons/hicolor/256x256/apps/X.png";

    /// **O quê:** HOME fictício absoluto, sem tocar o disco.
    ///
    /// **Onde:** testes de render.
    fn h() -> PathBuf {
        PathBuf::from("/h")
    }

    /// **O quê:** com `gui` a entrada vira a do app com janela.
    ///
    /// **Onde:** snapshots com janela.
    fn com_gui(mut e: EntradaAtalho, g: &str) -> EntradaAtalho {
        e.gui = Some(PathBuf::from(g));
        e
    }

    /// **O quê:** forma de terminal do market, byte a byte como o `render` original.
    ///
    /// **Onde:** snapshot `schematize_market_rs/src/nucleo/desktop.rs:81-102` (saída literal).
    #[test]
    fn snapshot_market_sem_gui() {
        let t = render(&market(&h()), ICONE).unwrap();
        assert_eq!(t, "[Desktop Entry]\nType=Application\nName=schematize Market\nGenericName=Instalar e remover programas\nComment=Linguagens, ferramentas de dev e os apps do ecossistema\nExec=/h/.cargo/bin/schematize-market list --wait\nIcon=/h/.local/share/icons/hicolor/256x256/apps/X.png\nTerminal=true\nCategories=Development;System;PackageManager;\nKeywords=instalar;install;programa;linguagem;runtime;pacote;market;\nStartupNotify=false\n");
    }

    /// **O quê:** forma com janela do market.
    ///
    /// **Onde:** snapshot do `render` original do market.
    #[test]
    fn snapshot_market_com_gui() {
        let e = com_gui(market(&h()), "/h/.cargo/bin/schematize-market-gui");
        assert_eq!(render(&e, ICONE).unwrap(), "[Desktop Entry]\nType=Application\nName=schematize Market\nGenericName=Instalar e remover programas\nComment=Linguagens, ferramentas de dev e os apps do ecossistema\nExec=/h/.cargo/bin/schematize-market-gui\nIcon=/h/.local/share/icons/hicolor/256x256/apps/X.png\nTerminal=false\nCategories=Development;System;PackageManager;\nKeywords=instalar;install;programa;linguagem;runtime;pacote;market;\nStartupNotify=false\n");
    }

    /// **O quê:** forma de terminal do overdev (`status`, com `StartupWMClass`).
    ///
    /// **Onde:** snapshot `schematize_overdev_rs/src/nucleo/desktop.rs:65-88`.
    #[test]
    fn snapshot_overdev_sem_gui() {
        assert_eq!(render(&overdev(&h()), ICONE).unwrap(), "[Desktop Entry]\nType=Application\nName=schematize Overdev\nGenericName=Run, checklist, perguntas\nComment=O progresso do run, o checklist e as perguntas que esperam você\nExec=/h/.cargo/bin/schematize-overdev status\nIcon=/h/.local/share/icons/hicolor/256x256/apps/X.png\nTerminal=true\nCategories=Development;Utility;\nKeywords=skill;skills;catalogo;catalog;claude;agente;agent;fork;\nStartupNotify=false\nStartupWMClass=schematize-overdev-gui\n");
    }

    /// **O quê:** forma com janela do overdev.
    ///
    /// **Onde:** snapshot do `render` original do overdev.
    #[test]
    fn snapshot_overdev_com_gui() {
        let e = com_gui(overdev(&h()), "/h/.cargo/bin/schematize-overdev-gui");
        assert_eq!(render(&e, ICONE).unwrap(), "[Desktop Entry]\nType=Application\nName=schematize Overdev\nGenericName=Run, checklist, perguntas\nComment=O progresso do run, o checklist e as perguntas que esperam você\nExec=/h/.cargo/bin/schematize-overdev-gui\nIcon=/h/.local/share/icons/hicolor/256x256/apps/X.png\nTerminal=false\nCategories=Development;Utility;\nKeywords=skill;skills;catalogo;catalog;claude;agente;agent;fork;\nStartupNotify=false\nStartupWMClass=schematize-overdev-gui\n");
    }

    /// **O quê:** o lançador do hub (`make_launcher`): sem `StartupNotify`, só janela.
    ///
    /// **Onde:** snapshot `schematize_market_rs/src/nucleo/plataforma.rs:456-463`.
    #[test]
    fn snapshot_market_launcher_hub() {
        let i = "/h/.local/share/icons/hicolor/256x256/apps/schematize.png";
        assert_eq!(render(&hub(&h()), i).unwrap(), "[Desktop Entry]\nType=Application\nName=schematize\nGenericName=Skills, overdev e mais\nComment=Skills, overdev e mais — schematize\nExec=/h/.cargo/bin/schematize-gui\nIcon=/h/.local/share/icons/hicolor/256x256/apps/schematize.png\nTerminal=false\nCategories=Development;Utility;\nKeywords=schematize;skills;overdev;claude;\nStartupWMClass=schematize-gui\n");
    }

    /// **O quê:** `Exec` absoluto nas duas formas, e a janela sem subcomando/`--wait`.
    ///
    /// **Onde:** porta de `o_exec_e_absoluto_com_e_sem_janela` e `com_a_janela_instalada...`.
    #[test]
    fn exec_absoluto_e_janela_sem_subcomando() {
        let sem = render(&market(&h()), ICONE).unwrap();
        assert!(sem.contains("Exec=/h/.cargo/bin/schematize-market list --wait\n"));
        let e = com_gui(market(&h()), "/h/.cargo/bin/schematize-market-gui");
        let com = render(&e, ICONE).unwrap();
        assert!(com.contains("Terminal=false") && !com.contains("--wait"), "{com}");
        assert!(!com.contains("Exec=schematize"), "relativo depende do PATH do DE");
    }

    /// **O quê:** os campos obrigatórios aparecem nas duas formas dos dois apps.
    ///
    /// **Onde:** porta de `tem_os_campos_obrigatorios_nas_duas_formas`.
    #[test]
    fn campos_obrigatorios_nas_duas_formas() {
        for base in [market(&h()), overdev(&h())] {
            for e in [base.clone(), com_gui(base, "/b/x-gui")] {
                let t = render(&e, ICONE).unwrap();
                for c in ["[Desktop Entry]", "Type=Application", "Name=", "Exec=", "Icon="] {
                    assert!(t.contains(c), "faltou {c}:\n{t}");
                }
                assert!(t.contains("Terminal=true") || t.contains("Terminal=false"));
            }
        }
    }

    /// **O quê:** `StartupWMClass` é o da JANELA, não o do CLI.
    ///
    /// **Onde:** porta de `a_classe_de_janela_e_a_da_janela` do overdev.
    #[test]
    fn classe_de_janela_e_a_da_janela() {
        let t = render(&overdev(&h()), ICONE).unwrap();
        assert!(t.contains("StartupWMClass=schematize-overdev-gui\n"));
        assert!(!t.contains("StartupWMClass=schematize-overdev\n"));
    }

    /// **O quê:** entradas hostis são recusadas: quebra de linha injetando chave, NUL, id com
    /// `..`/`/`, `;` em categoria, home relativo, nome vazio, sem exec.
    ///
    /// **Onde:** piso de segurança do render (injeção de chaves no `.desktop`).
    #[test]
    fn entradas_hostis_sao_recusadas() {
        type Muta = Box<dyn Fn(&mut EntradaAtalho)>;
        let ruins: Vec<Muta> = vec![
            Box::new(|e| e.nome = "a\nExec=/bin/rm".into()),
            Box::new(|e| e.nome = "a\0b".into()),
            Box::new(|e| e.nome = "   ".into()),
            Box::new(|e| e.comentario = Some("x\r\nTerminal=true".into())),
            Box::new(|e| e.id = "../../etc/x".into()),
            Box::new(|e| e.id = String::new()),
            Box::new(|e| e.icone_nome = "a/b".into()),
            Box::new(|e| e.legados = vec!["..".into()]),
            Box::new(|e| e.categorias = vec!["A;Exec=x".into()]),
            Box::new(|e| e.palavras_chave = vec!["k\n".into()]),
            Box::new(|e| e.startup_wm_class = Some("c\nX=1".into())),
            Box::new(|e| e.args_sem_gui = "a\nb".into()),
            Box::new(|e| e.home = PathBuf::from("rel")),
            Box::new(|e| e.bin = Some(PathBuf::from("/a\"b"))),
            Box::new(|e| e.bin = Some(PathBuf::from("/a$b"))),
            Box::new(|e| e.bin = Some(PathBuf::from("/a\nb"))),
            Box::new(|e| e.bin = None),
        ];
        for (i, f) in ruins.iter().enumerate() {
            let mut e = market(&h());
            f(&mut e);
            assert!(render(&e, ICONE).is_err(), "caso {i} deveria falhar: {e:?}");
        }
        assert!(render(&market(&h()), "i\ncon").is_err());
    }

    /// **O quê:** caminho com espaço vai entre aspas; `%` é duplicado; unicode passa intacto.
    ///
    /// **Onde:** deviation documentada: apps antigos não citavam (clique quebrava com espaço).
    #[test]
    fn exec_com_espaco_e_citado() {
        let mut e = market(&h());
        e.bin = Some(PathBuf::from("/h/Meu Usuário/100%/bin/m"));
        let t = render(&e, ICONE).unwrap();
        assert!(t.contains("Exec=\"/h/Meu Usuário/100%%/bin/m\" list --wait\n"), "{t}");
    }

    /// **O quê:** `bin` sem argumentos não deixa espaço sobrando no `Exec`.
    ///
    /// **Onde:** borda de `args_sem_gui` vazio.
    #[test]
    fn args_vazios_sem_espaco_final() {
        let mut e = market(&h());
        e.args_sem_gui.clear();
        assert!(render(&e, ICONE).unwrap().contains("Exec=/h/.cargo/bin/schematize-market\n"));
    }

    /// **O quê:** categorias/palavras vazias: `Categories=` vazia e `Keywords` omitida.
    ///
    /// **Onde:** borda das listas.
    #[test]
    fn listas_vazias() {
        let mut e = market(&h());
        e.categorias.clear();
        e.palavras_chave.clear();
        let t = render(&e, ICONE).unwrap();
        assert!(t.contains("Categories=\n") && !t.contains("Keywords"), "{t}");
    }

    /// **O quê:** a irmã ao lado ganha; sem irmã, `None` (nunca inventa caminho).
    ///
    /// **Onde:** porta de `resolver_gui_prefere_a_janela_irma`.
    #[test]
    fn resolver_gui_irma_prefere_a_irma() {
        let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/testes/irma");
        let d = d.join(std::process::id().to_string());
        std::fs::create_dir_all(&d).unwrap();
        let bin = d.join("schematize-market");
        assert_eq!(resolver_gui_irma(&bin, "schematize-market-gui"), None);
        std::fs::write(d.join("schematize-market-gui"), b"x").unwrap();
        assert_eq!(
            resolver_gui_irma(&bin, "schematize-market-gui"),
            Some(d.join("schematize-market-gui"))
        );
        assert_eq!(resolver_gui_irma(Path::new("/"), "x"), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
