//! Atalhos de menu: o contrato [`Atalhos`] e a implementação Unix ([`AtalhosUnix`]).
//!
//! **O quê:** instala/remove o `.desktop` em `<home>/.local/share/applications`, instala o ícone
//! em `hicolor` (+ fallbacks legados) e roda `update-desktop-database` / `gtk-update-icon-cache`
//! por um [`Executor`] injetável. Unifica `nucleo/desktop.rs` do market e do overdev e o
//! `make_launcher` do hub; o texto do `.desktop` vem de [`super::atalhos_render::render`].
//!
//! **Onde:** os apps (`desktop --install/--remover`, fim da instalação) via o trait.
//!
//! **Erro vs aviso:** falha na escrita PRIMÁRIA (`.desktop`, ou ícone se `icone_obrigatorio`)
//! é `Err`. Falha nos auxiliares (helpers do menu, ícones extras, remoção de legados) NÃO
//! invalida a instalação — o pior caso é o ícone aparecer só no próximo login —, mas nunca é
//! engolida: [`AtalhosUnix::instalar_com_avisos`] devolve a lista de avisos e o `instalar` do
//! trait os imprime em stderr.

pub use super::atalhos_render::{
    render, resolver_gui_irma, validar, EntradaAtalho, TAMANHO_ICONE_PRINCIPAL,
};

/// **O quê:** o contrato de quem sabe instalar/remover um atalho de menu na plataforma.
///
/// **Onde:** os apps dependem do trait; `AtalhosUnix` é a implementação de Linux/BSD.
pub trait Atalhos: Send + Sync {
    /// Instala (ou regrava) o atalho; falhas auxiliares saem em stderr como `aviso:`.
    fn instalar(&self, e: &EntradaAtalho) -> Result<(), String>;
    /// Remove o atalho; remover o que não existe é `Ok`.
    fn remover(&self, e: &EntradaAtalho) -> Result<(), String>;
}

#[cfg(unix)]
pub use unix::{AtalhosUnix, Executor, ProcessoReal};

#[cfg(unix)]
mod unix {
    use super::super::atalhos_render::{render, validar, EntradaAtalho, TAMANHO_ICONE_PRINCIPAL};
    use super::Atalhos;
    use std::path::{Path, PathBuf};

    /// Executa um comando auxiliar; injetável para os testes nunca rodarem o real.
    pub trait Executor: Send + Sync {
        /// Roda `prog args`; `Err` se não iniciou ou saiu com status != 0.
        fn executar(&self, prog: &str, args: &[String]) -> Result<(), String>;
    }

    /// Executor de produção: `std::process::Command`.
    pub struct ProcessoReal;

    impl Executor for ProcessoReal {
        /// **O quê:** roda o programa e converte spawn/status ruim em `Err`.
        ///
        /// **Onde:** `AtalhosUnix::auxiliar`, em produção.
        fn executar(&self, prog: &str, args: &[String]) -> Result<(), String> {
            let st = std::process::Command::new(prog)
                .args(args)
                .status()
                .map_err(|e| format!("não consegui rodar {prog}: {e}"))?;
            if st.success() {
                Ok(())
            } else {
                Err(format!("{prog} saiu com {st}"))
            }
        }
    }

    /// Atalhos freedesktop (Linux/BSD) sob `<home>/.local/share`.
    pub struct AtalhosUnix {
        runner: Box<dyn Executor>,
    }

    impl Default for AtalhosUnix {
        /// **O quê:** a de produção ([`ProcessoReal`]). **Onde:** `AtalhosUnix::default()`.
        fn default() -> Self {
            Self::com_executor(Box::new(ProcessoReal))
        }
    }

    impl AtalhosUnix {
        /// **O quê:** instância de produção. **Onde:** os apps.
        pub fn new() -> Self {
            Self::default()
        }

        /// **O quê:** instância com executor injetado. **Onde:** testes.
        pub fn com_executor(runner: Box<dyn Executor>) -> Self {
            Self { runner }
        }

        /// **O quê:** `<home>/.local/share/applications`. **Onde:** instalar/remover/testes.
        pub fn dir_apps(home: &Path) -> PathBuf {
            home.join(".local/share/applications")
        }

        /// **O quê:** o caminho do `.desktop` da entrada. **Onde:** instalar/remover/testes.
        pub fn arquivo_desktop(e: &EntradaAtalho) -> PathBuf {
            Self::dir_apps(&e.home).join(format!("{}.desktop", e.id))
        }

        /// **O quê:** roda um helper e, se falhar, acrescenta o motivo aos avisos.
        ///
        /// **Onde:** `instalar_com_avisos` e `remover_com_avisos`.
        fn auxiliar(&self, avisos: &mut Vec<String>, prog: &str, args: &[String]) {
            if let Err(m) = self.runner.executar(prog, args) {
                avisos.push(m);
            }
        }

        /// **O quê:** grava os PNGs em hicolor (e extras). `Err` só se `icone_obrigatorio`.
        ///
        /// **Onde:** `instalar_com_avisos`, ANTES do `.desktop` (ícone ausente = quadrado cinza).
        fn instalar_icones(
            e: &EntradaAtalho,
            avisos: &mut Vec<String>,
        ) -> Result<Option<PathBuf>, String> {
            let base = e.home.join(".local/share/icons/hicolor");
            let mut p256 = None;
            for (n, bytes) in &e.icones {
                let p = base.join(format!("{n}x{n}/apps/{}.png", e.icone_nome));
                match grava(&p, bytes) {
                    Ok(()) if *n == TAMANHO_ICONE_PRINCIPAL => p256 = Some(p),
                    Ok(()) => {}
                    Err(m) if e.icone_obrigatorio => return Err(format!("ícone: {m}")),
                    Err(m) => avisos.push(format!("ícone: {m}")),
                }
            }
            let principal = e.icones.iter().find(|(n, _)| *n == TAMANHO_ICONE_PRINCIPAL);
            if let (true, Some((_, bytes))) = (e.icone_extras, principal) {
                let n = &e.icone_nome;
                for x in [
                    format!(".local/share/pixmaps/{n}.png"),
                    format!(".icons/{n}.png"),
                    format!(".local/share/icons/{n}.png"),
                ] {
                    if let Err(m) = grava(&e.home.join(x), bytes) {
                        avisos.push(format!("ícone extra: {m}"));
                    }
                }
            }
            Ok(p256)
        }

        /// **O quê:** instala ícones + `.desktop` e devolve os AVISOS (falhas não fatais).
        ///
        /// **Onde:** `schematize-<app> desktop --install`, `install.sh`, fim da instalação do hub.
        ///
        /// **Ordem:** valida -> ícones -> `.desktop` -> legados -> `update-desktop-database` ->
        /// `gtk-update-icon-cache` (se `cache_icones`). `Icon=` é o PNG de 256px absoluto se ele
        /// existir; senão o nome do ícone.
        pub fn instalar_com_avisos(&self, e: &EntradaAtalho) -> Result<Vec<String>, String> {
            validar(e)?;
            let mut avisos = Vec::new();
            let p256 = Self::instalar_icones(e, &mut avisos)?;
            let icone = match p256 {
                Some(p) if p.is_file() => p.display().to_string(),
                _ => e.icone_nome.clone(),
            };
            let texto = render(e, &icone)?;
            let dir = Self::dir_apps(&e.home);
            std::fs::create_dir_all(&dir)
                .map_err(|x| format!("não consegui criar {}: {x}", dir.display()))?;
            let p = Self::arquivo_desktop(e);
            std::fs::write(&p, texto)
                .map_err(|x| format!("não consegui gravar {}: {x}", p.display()))?;
            for l in &e.legados {
                let lp = dir.join(format!("{l}.desktop"));
                match std::fs::remove_file(&lp) {
                    Ok(()) => {}
                    Err(x) if x.kind() == std::io::ErrorKind::NotFound => {}
                    Err(x) => avisos.push(format!("não consegui remover {}: {x}", lp.display())),
                }
            }
            self.auxiliar(&mut avisos, "update-desktop-database", &[dir.display().to_string()]);
            if e.cache_icones && !e.icones.is_empty() {
                let icons = e.home.join(".local/share/icons/hicolor").display().to_string();
                self.auxiliar(
                    &mut avisos,
                    "gtk-update-icon-cache",
                    &["-f".into(), "-t".into(), icons],
                );
            }
            Ok(avisos)
        }

        /// **O quê:** remove o `.desktop`. `(havia, avisos)`; ícones ficam (hicolor é compartilhada).
        ///
        /// **Onde:** `desktop --remover`.
        pub fn remover_com_avisos(&self, e: &EntradaAtalho) -> Result<(bool, Vec<String>), String> {
            validar(e)?;
            let p = Self::arquivo_desktop(e);
            if !p.exists() {
                return Ok((false, vec![]));
            }
            std::fs::remove_file(&p)
                .map_err(|x| format!("não consegui remover {}: {x}", p.display()))?;
            let mut avisos = Vec::new();
            let dir = Self::dir_apps(&e.home).display().to_string();
            self.auxiliar(&mut avisos, "update-desktop-database", &[dir]);
            Ok((true, avisos))
        }
    }

    /// **O quê:** cria os diretórios-pai e grava `bytes` em `p`; erro já com o caminho.
    ///
    /// **Onde:** `AtalhosUnix::instalar_icones`.
    fn grava(p: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)
                .map_err(|x| format!("não consegui criar {}: {x}", d.display()))?;
        }
        std::fs::write(p, bytes).map_err(|x| format!("não consegui gravar {}: {x}", p.display()))
    }

    /// **O quê:** imprime os avisos em stderr (nunca engolidos). **Onde:** o trait.
    fn avisar(avisos: &[String]) {
        for a in avisos {
            eprintln!("aviso: {a}");
        }
    }

    impl Atalhos for AtalhosUnix {
        /// **O quê:** `instalar_com_avisos` + avisos em stderr. **Onde:** os apps.
        fn instalar(&self, e: &EntradaAtalho) -> Result<(), String> {
            avisar(&self.instalar_com_avisos(e)?);
            Ok(())
        }

        /// **O quê:** `remover_com_avisos` + avisos em stderr. **Onde:** os apps.
        fn remover(&self, e: &EntradaAtalho) -> Result<(), String> {
            avisar(&self.remover_com_avisos(e)?.1);
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::super::atalhos_render::testes_util::{hub, market, overdev};
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        static N: AtomicUsize = AtomicUsize::new(0);

        /// **O quê:** HOME temporário único e limpo sob `target/testes`.
        ///
        /// **Onde:** todos os testes de disco (nunca o HOME real).
        fn tmp(nome: &str) -> PathBuf {
            let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/testes/atalhos").join(
                format!("{nome}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)),
            );
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).expect("tmp");
            d
        }

        /// Executor falso: grava as chamadas e falha para os programas em `falha`.
        struct Falso {
            chamadas: Arc<Mutex<Vec<String>>>,
            falha: Vec<&'static str>,
        }

        impl Executor for Falso {
            /// **O quê:** registra `prog args` e falha se `prog` está em `falha`.
            ///
            /// **Onde:** testes, no lugar do comando real.
            fn executar(&self, prog: &str, args: &[String]) -> Result<(), String> {
                self.chamadas.lock().expect("lock").push(format!("{prog} {}", args.join(" ")));
                if self.falha.contains(&prog) {
                    return Err(format!("{prog} falhou"));
                }
                Ok(())
            }
        }

        /// **O quê:** `AtalhosUnix` com executor falso + o log de chamadas.
        ///
        /// **Onde:** testes.
        fn falso(falha: Vec<&'static str>) -> (AtalhosUnix, Arc<Mutex<Vec<String>>>) {
            let c = Arc::new(Mutex::new(vec![]));
            (AtalhosUnix::com_executor(Box::new(Falso { chamadas: c.clone(), falha })), c)
        }

        /// **O quê:** instalar e remover (market) deixa o diretório como estava; regravar troca a
        /// forma; o ícone existe antes do `.desktop`.
        ///
        /// **Onde:** porta de `instalar_e_remover_volta_ao_estado_anterior` e `instala_e_remove_no_disco`.
        #[test]
        fn instalar_e_remover_ida_e_volta() {
            let h = tmp("ida");
            let (a, calls) = falso(vec![]);
            let mut e = market(&h);
            assert!(a.instalar_com_avisos(&e).unwrap().is_empty());
            let p = AtalhosUnix::arquivo_desktop(&e);
            let t = std::fs::read_to_string(&p).unwrap();
            assert!(t.contains("Exec=/h/.cargo/bin/schematize-market list --wait"), "{t}");
            let png = h.join(".local/share/icons/hicolor/256x256/apps/schematize-market.png");
            assert_eq!(std::fs::read(&png).unwrap(), b"p256");
            assert!(t.contains(&format!("Icon={}\n", png.display())), "{t}");
            for x in [".local/share/pixmaps", ".icons", ".local/share/icons"] {
                assert!(h.join(x).join("schematize-market.png").is_file(), "{x}");
            }
            e.gui = Some("/h/.cargo/bin/schematize-market-gui".into());
            a.instalar(&e).unwrap();
            assert!(std::fs::read_to_string(&p).unwrap().contains("Terminal=false"));
            let (havia, av) = a.remover_com_avisos(&e).unwrap();
            assert!(havia && av.is_empty() && !p.exists());
            assert!(!a.remover_com_avisos(&e).unwrap().0);
            a.remover(&e).unwrap();
            assert!(calls.lock().unwrap().iter().all(|c| c.starts_with("update-desktop-database")));
            std::fs::remove_dir_all(&h).unwrap();
        }

        /// **O quê:** overdev grava o `.desktop` e o hicolor de 16 e 256.
        ///
        /// **Onde:** porta do teste de disco do overdev.
        #[test]
        fn overdev_no_disco() {
            let h = tmp("od");
            let (a, _) = falso(vec![]);
            a.instalar(&overdev(&h)).unwrap();
            let p = AtalhosUnix::dir_apps(&h).join("schematize-overdev.desktop");
            assert!(p.is_file());
            assert!(h
                .join(".local/share/icons/hicolor/16x16/apps/schematize-overdev.png")
                .is_file());
            std::fs::remove_dir_all(&h).unwrap();
        }

        /// **O quê:** hub: cache de ícones roda após o menu, legado `overflow-gui` é removido, e
        /// ícone ausente cai para o nome.
        ///
        /// **Onde:** porta do `make_launcher`.
        #[test]
        fn hub_remove_legado_e_atualiza_cache() {
            let h = tmp("hub");
            let (a, calls) = falso(vec![]);
            let dir = AtalhosUnix::dir_apps(&h);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("overflow-gui.desktop"), "x").unwrap();
            assert!(a.instalar_com_avisos(&hub(&h)).unwrap().is_empty());
            assert!(!dir.join("overflow-gui.desktop").exists());
            let c = calls.lock().unwrap().clone();
            assert!(c[0].starts_with("update-desktop-database "), "{c:?}");
            assert!(c[1].starts_with("gtk-update-icon-cache -f -t "), "{c:?}");
            let mut sem = hub(&h);
            sem.icones.clear();
            a.instalar(&sem).unwrap();
            let t = std::fs::read_to_string(dir.join("schematize-gui.desktop")).unwrap();
            assert!(t.contains("Icon=schematize\n"), "{t}");
            std::fs::remove_dir_all(&h).unwrap();
        }

        /// **O quê:** falha dos helpers vira AVISO (não `Err`), um por helper.
        ///
        /// **Onde:** contrato de não engolir erro (piso 4).
        #[test]
        fn falha_de_helper_vira_aviso() {
            let h = tmp("av");
            let (a, _) = falso(vec!["update-desktop-database", "gtk-update-icon-cache"]);
            let av = a.instalar_com_avisos(&hub(&h)).unwrap();
            assert_eq!(av.len(), 2, "{av:?}");
            assert!(AtalhosUnix::arquivo_desktop(&hub(&h)).is_file());
            std::fs::remove_dir_all(&h).unwrap();
        }

        /// **O quê:** ícone obrigatório com destino impossível é `Err` e NÃO grava `.desktop`;
        /// não obrigatório vira aviso.
        ///
        /// **Onde:** erro primário vs auxiliar.
        #[test]
        fn icone_obrigatorio_falha_e_nao_grava_desktop() {
            let h = tmp("ic");
            std::fs::write(h.join(".local"), b"arquivo, nao dir").unwrap();
            let (a, _) = falso(vec![]);
            let e = market(&h);
            let r = a.instalar_com_avisos(&e);
            assert!(r.unwrap_err().contains("ícone"));
            let mut e = hub(&h);
            e.icones = vec![(256, b"x".to_vec())];
            e.cache_icones = false;
            // `.desktop` também não pode ser criado (.local é arquivo): Err primário.
            assert!(a.instalar_com_avisos(&e).is_err());
            std::fs::remove_dir_all(&h).unwrap();
        }

        /// **O quê:** entrada hostil falha ANTES de tocar o disco ou rodar helper.
        ///
        /// **Onde:** validação precede efeitos.
        #[test]
        fn hostil_nao_toca_disco_nem_helpers() {
            let h = tmp("ho");
            let (a, calls) = falso(vec![]);
            let mut e = market(&h);
            e.id = "../fora".into();
            assert!(a.instalar_com_avisos(&e).is_err());
            assert!(a.remover_com_avisos(&e).is_err());
            assert!(calls.lock().unwrap().is_empty());
            assert!(!h.join(".local").exists());
            std::fs::remove_dir_all(&h).unwrap();
        }

        /// **O quê:** executor real: programa inexistente e status != 0 viram `Err`.
        ///
        /// **Onde:** `ProcessoReal` (sem rede, sem tocar o HOME).
        #[test]
        fn processo_real_reporta_falhas() {
            assert!(ProcessoReal.executar("programa-que-nao-existe-xyz", &[]).is_err());
            assert!(ProcessoReal.executar("false", &[]).is_err());
            assert!(ProcessoReal.executar("true", &[]).is_ok());
        }
    }
}
