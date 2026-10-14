//! Abrir um comando/script num emulador de terminal gráfico, desacoplado do app.
//!
//! **O quê:** o seam [`Terminal`], a implementação [`TerminalUnix`], as listas de emuladores
//! por perfil ([`Emuladores`]), o montador de argv ([`argv`]) e os construtores PUROS dos
//! scripts de cada chamador (`corpo_*`). Unifica quatro cópias:
//! `schematize_market_rs/src/nucleo/bin.rs:131-170` (`abrir_comando_no_terminal` +
//! `spawn_no_terminal`), `schematize_overdev_rs/src/agentrun/lancador.rs:255-310` e `335-395`,
//! `schematize_overdev_rs/src/nucleo/agente.rs:27-110` e `schematize_overdev_rs/src/gui/terminal.rs:30-75`.
//!
//! **Onde:** botões «abrir no terminal» (vps, projeto), lançamento do agente (`overdev`,
//! `rerun`) e a janela de restaurar snapshot. Descoberta (`existe`) e spawn são injetáveis:
//! os testes nunca abrem terminal de verdade.
//!
//! **Conteúdo dos scripts é contrato:** os `corpo_*` reproduzem BYTE A BYTE o que cada
//! chamador gerava; os testes `snapshot_*` fixam as strings literais.

use std::path::{Path, PathBuf};

use super::abridor::{no_path, Lancar};

/// **O quê:** abre um script (ou comando) num terminal. **Onde:** injetado nos chamadores;
/// testes usam um falso.
pub trait Terminal: Send + Sync {
    /// **O quê:** grava (se houver arquivo) e abre `s` no primeiro emulador disponível.
    /// **Onde:** todos os fluxos «abrir no terminal».
    fn abrir_script(&self, s: &ScriptTerminal) -> Result<(), String>;
}

/// **O quê:** qual lista/estilo de emuladores um chamador usa (cada cópia original tinha a
/// sua e a ordem/flags são parte do comportamento). **Onde:** [`ScriptTerminal::emuladores`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emuladores {
    /// Market + lancador do overdev: 7 terminais, `-e bash script` / `-- bash script`.
    Casa,
    /// `nucleo/agente.rs` do overdev: lista própria (com `wezterm`, sem `x-terminal-emulator`).
    Agente,
    /// `gui/terminal.rs`: flag por emulador e `bash -c <comando>` direto, sem arquivo.
    Gui,
}

/// **O quê:** o que o terminal executa. **Onde:** [`ScriptTerminal::entrega`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entrega {
    /// Grava `corpo` em `caminho` (com `modo` unix, ex. `0o700`) e roda `bash caminho`.
    Arquivo { caminho: PathBuf, corpo: String, modo: u32 },
    /// Roda `bash -c comando` (sem arquivo temporário).
    Inline(String),
}

/// **O quê:** descreve UM lançamento. O diretório de trabalho, o `PATH`, o env e o «manter a
/// janela aberta» vivem DENTRO do `corpo` (`cd`, `export`, `read`/`exec $SHELL`), montado por
/// um `corpo_*`. **Onde:** passado a [`Terminal::abrir_script`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptTerminal {
    /// O que rodar (arquivo gravado ou comando inline).
    pub entrega: Entrega,
    /// Perfil de emuladores.
    pub emuladores: Emuladores,
}

/// **O quê:** a implementação real para unix. **Onde:** composição do app.
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalUnix;

/// Lista da casa (market + lancador), em ordem de preferência.
const CASA: [&str; 7] = [
    "konsole",
    "gnome-terminal",
    "xfce4-terminal",
    "x-terminal-emulator",
    "alacritty",
    "kitty",
    "xterm",
];

/// Lista do agente do overdev.
const AGENTE: [&str; 7] =
    ["konsole", "gnome-terminal", "xfce4-terminal", "alacritty", "kitty", "wezterm", "xterm"];

/// Lista da GUI: emulador e a flag que recebe o comando (`kitty` não tem flag).
const GUI: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("xfce4-terminal", &["-x"]),
    ("tilix", &["-e"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("xterm", &["-e"]),
];

/// **O quê:** os nomes de emulador do perfil, na ordem de tentativa. **Onde:**
/// [`escolher`] e os testes.
pub fn nomes(e: Emuladores) -> Vec<&'static str> {
    match e {
        Emuladores::Casa => CASA.to_vec(),
        Emuladores::Agente => AGENTE.to_vec(),
        Emuladores::Gui => GUI.iter().map(|(n, _)| *n).collect(),
    }
}

/// **O quê:** o primeiro emulador do perfil para o qual `existe` diz `true`. **Onde:**
/// [`abrir_com`].
pub fn escolher(e: Emuladores, existe: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    nomes(e).into_iter().find(|t| existe(t))
}

/// **O quê:** o argv (sem o programa) de `term` para `entrega`. Pura. `--` para
/// gnome/xfce4 e `-e` para os demais (errar o separador abre o terminal VAZIO); a GUI usa a
/// flag da própria tabela e `bash -c`. **Onde:** [`abrir_com`] e os snapshots.
pub fn argv(e: Emuladores, term: &str, entrega: &Entrega) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    match (e, entrega) {
        (Emuladores::Gui, Entrega::Inline(c)) => {
            let pre = GUI.iter().find(|(n, _)| *n == term).map(|(_, p)| *p).unwrap_or(&[]);
            v.extend(pre.iter().map(|s| s.to_string()));
            v.extend(["bash".to_string(), "-c".to_string(), c.clone()]);
        }
        (_, Entrega::Arquivo { caminho, .. }) => {
            v.push(
                if matches!(term, "gnome-terminal" | "xfce4-terminal") { "--" } else { "-e" }
                    .into(),
            );
            v.push("bash".into());
            v.push(caminho.display().to_string());
        }
        (_, Entrega::Inline(c)) => {
            v.extend(["-e".to_string(), "bash".to_string(), "-c".to_string(), c.clone()]);
        }
    }
    v
}

/// **O quê:** grava o script e aplica `modo` (unix). Erros propagam. **Onde:**
/// [`abrir_com`]; era `fs::write` + `set_permissions` ignorado nas cópias originais.
pub fn gravar_script(caminho: &Path, corpo: &str, modo: u32) -> Result<(), String> {
    if let Some(pai) = caminho.parent() {
        std::fs::create_dir_all(pai)
            .map_err(|e| format!("não consegui criar {}: {e}", pai.display()))?;
    }
    std::fs::write(caminho, corpo)
        .map_err(|e| format!("não consegui gravar {}: {e}", caminho.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(caminho, std::fs::Permissions::from_mode(modo))
            .map_err(|e| format!("não consegui ajustar permissão de {}: {e}", caminho.display()))?;
    }
    #[cfg(not(unix))]
    let _ = modo;
    Ok(())
}

/// **O quê:** o fluxo completo com `existe` e `lancar(programa, argv)` injetados: escolhe o
/// terminal, grava o script, lança. Devolve o NOME do terminal (mensagem «abri no konsole»).
/// **Onde:** [`TerminalUnix::abrir_script`] e os testes.
pub fn abrir_com(
    s: &ScriptTerminal,
    existe: &dyn Fn(&str) -> bool,
    lancar: Lancar,
) -> Result<String, String> {
    let term = escolher(s.emuladores, existe).ok_or_else(|| {
        format!("nenhum terminal encontrado ({}).", nomes(s.emuladores).join("/"))
    })?;
    if let Entrega::Arquivo { caminho, corpo, modo } = &s.entrega {
        gravar_script(caminho, corpo, *modo)?;
    }
    lancar(term, &argv(s.emuladores, term, &s.entrega))
        .map_err(|e| format!("abrir terminal `{term}`: {e}"))?;
    Ok(term.to_string())
}

/// **O quê:** spawn real: stdio nulo e grupo de processos próprio (sobrevive ao app).
/// **Onde:** [`TerminalUnix::abrir_script`]. Efeito: cria processo.
fn lancar_real(prog: &str, args: &[String]) -> Result<(), String> {
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

impl TerminalUnix {
    /// **O quê:** como [`Terminal::abrir_script`], mas devolve o nome do terminal usado.
    /// **Onde:** chamadores que mostram «aberto no X».
    pub fn abrir_nomeado(&self, s: &ScriptTerminal) -> Result<String, String> {
        abrir_com(s, &no_path, &lancar_real)
    }
}

impl Terminal for TerminalUnix {
    /// **O quê:** abre `s` no terminal real. **Onde:** todos os fluxos «abrir no terminal».
    fn abrir_script(&self, s: &ScriptTerminal) -> Result<(), String> {
        self.abrir_nomeado(s).map(|_| ())
    }
}

/// **O quê:** `ScriptTerminal` de um COMANDO que vira `exec` (market `abrir_comando_no_terminal`,
/// lancador idem): `#!/bin/sh\nexec {comando}\n`, modo `0o700`. **Onde:** `vps::conexao`.
pub fn script_comando(dir: &Path, pid: u32, comando: &str) -> ScriptTerminal {
    ScriptTerminal {
        entrega: Entrega::Arquivo {
            caminho: dir.join(format!("term-{pid}.sh")),
            corpo: corpo_exec(comando),
            modo: 0o700,
        },
        emuladores: Emuladores::Casa,
    }
}

/// **O quê:** corpo do script do market/lancador: o `exec` faz o terminal ficar com o processo
/// de verdade (fechar a janela encerra a sessão). **Onde:** [`script_comando`].
pub fn corpo_exec(comando: &str) -> String {
    format!("#!/bin/sh\nexec {comando}\n")
}

/// **O quê:** a linha do `claude` no shell interativo: absoluto se achado, senão aviso.
/// **Onde:** [`corpo_projeto_interativo`] (lancador `abrir_terminal_no_projeto`).
pub fn linha_claude(claude: Option<&Path>) -> String {
    match claude {
        Some(c) => format!("if [ -x {c:?} ]; then\n  {c:?} --dangerously-skip-permissions\nfi\n"),
        None => "echo '[schematize] o CLI `claude` nao esta no PATH — instale em claude.ai/code.'\necho '[schematize] abrindo so o shell neste projeto.'\n".to_string(),
    }
}

/// **O quê:** corpo do terminal interativo no projeto (lancador:350): PATH, `cd`, claude e
/// `exec $SHELL -i` (a janela não fecha). **Onde:** `abrir_terminal_no_projeto`.
pub fn corpo_projeto_interativo(projeto: &Path, claude: Option<&Path>) -> String {
    format!(
        "#!/usr/bin/env bash\nexport PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.claude/local:$PATH\"\ncd {proj:?} || exit 1\n{linha_claude}echo\necho '[schematize] voce esta em {mostra} — o shell segue aberto.'\nexec \"${{SHELL:-/bin/bash}}\" -i\n",
        proj = projeto,
        linha_claude = linha_claude(claude),
        mostra = projeto.display(),
    )
}

/// **O quê:** corpo do wrapper do prompt do overdev (lancador:384): roda o claude com o prompt
/// lido de arquivo e espera Enter. `modelo` já vem entre aspas do bash. **Onde:**
/// `launch_prompt_in_terminal`.
pub fn corpo_claude_prompt(
    projeto: &Path,
    claude: &Path,
    modelo: &str,
    promptfile: &Path,
) -> String {
    format!(
        "#!/usr/bin/env bash\nexport PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.claude/local:$PATH\"\ncd {proj:?} || exit 1\n{claude:?} --dangerously-skip-permissions --model {modelo} \"$(cat {pf:?})\"\necho\nread -rp '[overdev encerrado — Enter para fechar] '\n",
        proj = projeto,
        claude = claude,
        pf = promptfile,
    )
}

/// **O quê:** corpo do wrapper do agente `rerun` (agente.rs:62-72): como o anterior, sem
/// `--model` e com outra mensagem. **Onde:** `nucleo::agente::abrir`.
pub fn corpo_agente(projeto: &Path, claude: &Path, promptfile: &Path) -> String {
    format!(
        "#!/usr/bin/env bash\nexport PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.claude/local:$PATH\"\ncd {proj:?} || exit 1\n{claude:?} --dangerously-skip-permissions \"$(cat {pf:?})\"\necho\nread -rp '[agente encerrado — Enter para fechar] '\n",
        proj = projeto,
        claude = claude,
        pf = promptfile,
    )
}

/// **O quê:** `ScriptTerminal` da GUI: o comando vai inline em `bash -c`. **Onde:** ações da
/// janela que mudam estado (restaurar snapshot).
pub fn script_gui(comando: &str) -> ScriptTerminal {
    ScriptTerminal { entrega: Entrega::Inline(comando.to_string()), emuladores: Emuladores::Gui }
}

/// **O quê:** `ScriptTerminal` de um wrapper em arquivo (lancador/agente) com `modo` `0o755`.
/// **Onde:** `launch_prompt_in_terminal`, `abrir_terminal_no_projeto`, `agente::abrir`.
pub fn script_arquivo(caminho: PathBuf, corpo: String, e: Emuladores) -> ScriptTerminal {
    ScriptTerminal { entrega: Entrega::Arquivo { caminho, corpo, modo: 0o755 }, emuladores: e }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// **O quê:** `Fn` falso de existência: só `ok` existe. **Onde:** testes.
    fn so(ok: &'static str) -> impl Fn(&str) -> bool {
        move |b| b == ok
    }

    /// **O quê:** dir temporário único em `target/testes`. **Onde:** testes de gravação.
    fn tmp(nome: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/testes")
            .join(format!("terminal-{nome}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// **O quê:** projeto fixo para os snapshots. **Onde:** testes.
    fn proj() -> &'static Path {
        Path::new("/home/u/meu projeto")
    }

    /// **O quê:** o script do market, byte a byte. **Onde:** teste.
    #[test]
    fn snapshot_market_comando() {
        assert_eq!(
            corpo_exec("ssh -p 22 root@10.0.0.1"),
            "#!/bin/sh\nexec ssh -p 22 root@10.0.0.1\n"
        );
        let s = script_comando(Path::new("/h/.app/run"), 4242, "ssh a@b");
        assert_eq!(
            s.entrega,
            Entrega::Arquivo {
                caminho: PathBuf::from("/h/.app/run/term-4242.sh"),
                corpo: "#!/bin/sh\nexec ssh a@b\n".to_string(),
                modo: 0o700
            }
        );
    }

    /// **O quê:** argv por emulador da lista Casa (market/lancador). **Onde:** teste.
    #[test]
    fn snapshot_market_argv_por_emulador() {
        let e =
            Entrega::Arquivo { caminho: "/r/term-1.sh".into(), corpo: String::new(), modo: 0o700 };
        let casos: [(&str, [&str; 3]); 7] = [
            ("konsole", ["-e", "bash", "/r/term-1.sh"]),
            ("gnome-terminal", ["--", "bash", "/r/term-1.sh"]),
            ("xfce4-terminal", ["--", "bash", "/r/term-1.sh"]),
            ("x-terminal-emulator", ["-e", "bash", "/r/term-1.sh"]),
            ("alacritty", ["-e", "bash", "/r/term-1.sh"]),
            ("kitty", ["-e", "bash", "/r/term-1.sh"]),
            ("xterm", ["-e", "bash", "/r/term-1.sh"]),
        ];
        for (t, esp) in casos {
            assert_eq!(argv(Emuladores::Casa, t, &e), esp.to_vec(), "{t}");
        }
        assert_eq!(
            nomes(Emuladores::Casa),
            [
                "konsole",
                "gnome-terminal",
                "xfce4-terminal",
                "x-terminal-emulator",
                "alacritty",
                "kitty",
                "xterm"
            ]
        );
    }

    /// **O quê:** wrapper do lancador: terminal interativo, com e sem claude. **Onde:** teste.
    #[test]
    fn snapshot_overdev_lancador_projeto_interativo() {
        let com = corpo_projeto_interativo(proj(), Some(Path::new("/h/.local/bin/claude")));
        assert_eq!(
            com,
            "#!/usr/bin/env bash\nexport PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.claude/local:$PATH\"\ncd \"/home/u/meu projeto\" || exit 1\nif [ -x \"/h/.local/bin/claude\" ]; then\n  \"/h/.local/bin/claude\" --dangerously-skip-permissions\nfi\necho\necho '[schematize] voce esta em /home/u/meu projeto — o shell segue aberto.'\nexec \"${SHELL:-/bin/bash}\" -i\n"
        );
        let sem = corpo_projeto_interativo(proj(), None);
        assert!(sem.contains("echo '[schematize] o CLI `claude` nao esta no PATH — instale em claude.ai/code.'\necho '[schematize] abrindo so o shell neste projeto.'\necho\n"));
        assert!(!sem.contains("dangerously"));
    }

    /// **O quê:** wrapper do prompt do overdev (lancador). **Onde:** teste.
    #[test]
    fn snapshot_overdev_lancador_prompt() {
        let sh = corpo_claude_prompt(
            proj(),
            Path::new("/h/.local/bin/claude"),
            "'opus'",
            Path::new("/p/.schematize/overdev/prompt-1-0.txt"),
        );
        assert_eq!(
            sh,
            "#!/usr/bin/env bash\nexport PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.claude/local:$PATH\"\ncd \"/home/u/meu projeto\" || exit 1\n\"/h/.local/bin/claude\" --dangerously-skip-permissions --model 'opus' \"$(cat \"/p/.schematize/overdev/prompt-1-0.txt\")\"\necho\nread -rp '[overdev encerrado — Enter para fechar] '\n"
        );
    }

    /// **O quê:** wrapper do agente `rerun` e a lista própria dele. **Onde:** teste.
    #[test]
    fn snapshot_overdev_agente_wrapper_e_argv() {
        let sh = corpo_agente(
            proj(),
            Path::new("/h/.local/bin/claude"),
            Path::new("/p/skills/prompt-1-0.txt"),
        );
        assert_eq!(
            sh,
            "#!/usr/bin/env bash\nexport PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$HOME/.claude/local:$PATH\"\ncd \"/home/u/meu projeto\" || exit 1\n\"/h/.local/bin/claude\" --dangerously-skip-permissions \"$(cat \"/p/skills/prompt-1-0.txt\")\"\necho\nread -rp '[agente encerrado — Enter para fechar] '\n"
        );
        assert_eq!(
            nomes(Emuladores::Agente),
            [
                "konsole",
                "gnome-terminal",
                "xfce4-terminal",
                "alacritty",
                "kitty",
                "wezterm",
                "xterm"
            ]
        );
        let e = Entrega::Arquivo { caminho: "/p/run-1-0.sh".into(), corpo: sh, modo: 0o755 };
        assert_eq!(argv(Emuladores::Agente, "wezterm", &e), ["-e", "bash", "/p/run-1-0.sh"]);
        assert_eq!(argv(Emuladores::Agente, "gnome-terminal", &e), ["--", "bash", "/p/run-1-0.sh"]);
        assert_eq!(argv(Emuladores::Agente, "konsole", &e), ["-e", "bash", "/p/run-1-0.sh"]);
    }

    /// **O quê:** argv da GUI por emulador (`bash -c <cmd>`, kitty sem flag). **Onde:** teste.
    #[test]
    fn snapshot_overdev_gui_argv_por_emulador() {
        let s = script_gui("cd 'a b' && ls");
        let casos: [(&str, &[&str]); 8] = [
            ("x-terminal-emulator", &["-e"]),
            ("konsole", &["-e"]),
            ("gnome-terminal", &["--"]),
            ("xfce4-terminal", &["-x"]),
            ("tilix", &["-e"]),
            ("kitty", &[]),
            ("alacritty", &["-e"]),
            ("xterm", &["-e"]),
        ];
        for (t, pre) in casos {
            let mut esp: Vec<String> = pre.iter().map(|x| x.to_string()).collect();
            esp.extend(["bash", "-c", "cd 'a b' && ls"].map(String::from));
            assert_eq!(argv(Emuladores::Gui, t, &s.entrega), esp, "{t}");
        }
        assert_eq!(nomes(Emuladores::Gui)[0], "x-terminal-emulator");
    }

    /// **O quê:** a escolha respeita a ordem de cada perfil. **Onde:** teste.
    #[test]
    fn escolha_segue_a_ordem_do_perfil() {
        let tudo = |_: &str| true;
        assert_eq!(escolher(Emuladores::Casa, &tudo), Some("konsole"));
        assert_eq!(escolher(Emuladores::Gui, &tudo), Some("x-terminal-emulator"));
        assert_eq!(escolher(Emuladores::Agente, &so("wezterm")), Some("wezterm"));
        assert_eq!(escolher(Emuladores::Casa, &so("wezterm")), None);
        assert_eq!(escolher(Emuladores::Casa, &|_| false), None);
    }

    /// **O quê:** listas sãs: sem vazio nem duplicado. **Onde:** teste.
    #[test]
    fn listas_sao_sas() {
        for e in [Emuladores::Casa, Emuladores::Agente, Emuladores::Gui] {
            let n = nomes(e);
            assert!(n.iter().all(|x| !x.is_empty()));
            let mut o = n.clone();
            o.sort_unstable();
            o.dedup();
            assert_eq!(o.len(), n.len(), "{e:?}");
        }
    }

    /// **O quê:** sem terminal o erro lista os candidatos e nada é gravado nem lançado.
    /// **Onde:** teste.
    #[test]
    fn sem_terminal_erra_sem_gravar() {
        let d = tmp("sem");
        let c = d.join("x.sh");
        let s = script_arquivo(c.clone(), "#!/bin/sh\n".into(), Emuladores::Casa);
        let e = abrir_com(&s, &|_| false, &|_, _| Err("não deveria".into())).unwrap_err();
        assert!(e.contains("nenhum terminal") && e.contains("konsole"), "{e}");
        assert!(!c.exists());
    }

    /// **O quê:** grava com o modo, lança o argv certo e devolve o nome. **Onde:** teste.
    #[test]
    fn grava_com_modo_e_lanca() {
        let d = tmp("ok");
        let c = d.join("sub dir/term é.sh");
        let s = script_comando(&d, 7, "echo 'a b' \"$(x)\"");
        let s = ScriptTerminal {
            entrega: match s.entrega {
                Entrega::Arquivo { corpo, modo, .. } => {
                    Entrega::Arquivo { caminho: c.clone(), corpo, modo }
                }
                o => o,
            },
            ..s
        };
        let vistas = Mutex::new(Vec::new());
        let nome = abrir_com(&s, &so("gnome-terminal"), &|p, a| {
            vistas.lock().unwrap().push((p.to_string(), a.to_vec()));
            Ok(())
        })
        .unwrap();
        assert_eq!(nome, "gnome-terminal");
        let v = vistas.into_inner().unwrap();
        assert_eq!(
            v,
            vec![(
                "gnome-terminal".to_string(),
                vec!["--".into(), "bash".into(), c.display().to_string()]
            )]
        );
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "#!/bin/sh\nexec echo 'a b' \"$(x)\"\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&c).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    /// **O quê:** falha do spawn e do disco são propagadas (não engolidas). **Onde:** teste.
    #[test]
    fn falhas_sao_propagadas() {
        let d = tmp("falha");
        let s = script_arquivo(d.join("a.sh"), "x".into(), Emuladores::Casa);
        let e = abrir_com(&s, &so("xterm"), &|_, _| Err("boom".into())).unwrap_err();
        assert!(e.contains("xterm") && e.contains("boom"), "{e}");
        // caminho cujo pai é um ARQUIVO: não dá para criar o diretório.
        std::fs::write(d.join("arquivo"), "x").unwrap();
        let s = script_arquivo(d.join("arquivo/a.sh"), "x".into(), Emuladores::Casa);
        let e = abrir_com(&s, &so("xterm"), &|_, _| Ok(())).unwrap_err();
        assert!(e.contains("não consegui"), "{e}");
    }

    /// **O quê:** comando hostil inline vai como UM argumento, sem shell nosso. **Onde:** teste.
    #[test]
    fn comando_hostil_inline_e_um_argumento() {
        for c in ["-rf /", "a\nb", "é ñ 日本", "$(rm -rf ~)", "x\0y", "'; ls #"] {
            let s = script_gui(c);
            let a = argv(Emuladores::Gui, "xterm", &s.entrega);
            assert_eq!(a.last().map(String::as_str), Some(c));
            assert_eq!(a.len(), 4, "{c:?}");
        }
    }

    /// **O quê:** caminho com espaço/aspas/unicode no `Debug` do corpo: documenta o escape
    /// herdado (`{:?}` escapa `"` e `\`). **Onde:** teste.
    #[test]
    fn corpo_com_projeto_hostil_escapa_como_o_original() {
        let sh = corpo_projeto_interativo(Path::new("/p/\"q\" $(x)\né"), None);
        assert!(sh.contains("cd \"/p/\\\"q\\\" $(x)\\né\" || exit 1\n"), "{sh}");
    }

    /// **O quê:** o `TerminalUnix` real implementa o trait (objeto dinâmico compila).
    /// **Onde:** teste.
    #[test]
    fn terminal_unix_e_objeto_dinamico() {
        let _t: Box<dyn Terminal> = Box::new(TerminalUnix);
    }
}
