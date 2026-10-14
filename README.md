# schematize_plataforma

Infraestrutura de plataforma **compartilhada** do ecossistema schematize: o que estava copiado em
10 repos, num lugar só. **Std-only, zero dependências** (as janelas Slint dependem só de `slint`),
sem domínio (piso 6). Licença MIT.

## Features

| feature | módulos | default |
|---|---|---|
| `parasita` | `parasita` — garante `schematize-market` (+`-gui`) em `~/.cargo/bin` | sim |
| `base` | `procedencia`, `bin`, `arquivo` | sim |

| `seams` | `seams` — traits `Shell, Rede, Substituicao, Atalhos, PathSetup, Terminal, Abridor, Processos` + `Plataforma::unix()` (linux/freebsd/macos) | não |

`home` (resolução de `HOME`) compila com qualquer uma delas. Quem só quer o parasita:
`default-features = false, features = ["parasita"]`.

## Pin (política)

Dependência **git pinada por tag** — nunca `branch` nem `rev` solto:

```toml
schematize_plataforma = { git = "https://github.com/schematizeme/schematize_plataforma_rs", tag = "v0.2.0" }
# só o parasita:
schematize_plataforma = { git = "https://github.com/schematizeme/schematize_plataforma_rs", tag = "v0.2.0", default-features = false, features = ["parasita"] }
```

Subir o pin é um commit explícito em cada app. O CI de cada app deve ter o teste "pin == última tag".
Dev local antes de publicar: `schematize_app/.cargo/config.toml` com `[patch."https://github.com/schematizeme/schematize_plataforma_rs"]`
apontando para o checkout (fora de qualquer repo).

## `parasita`

Contrato: `.schematize/scratch/market-parasita/CONTRATO.md`. No `main`, depois do parse de args:

```rust
use schematize_plataforma::parasita::{self, Config, HOOKS_OVERDEV};

const CFG: Config = Config::new("schematize").com_hooks(HOOKS_OVERDEV); // cli/overdev
// apps sem hooks: Config::new("schematize-optimizer")
parasita::garantir(&CFG);
```

API principal:

- `Config::new(app) -> Config`, `.com_hooks(&[&[&str]])`, `.com_timeout(Duration)`. Campos: `app` (prefixo das
  mensagens), `hooks` (prefixos de args; `|` separa alternativas — `&["overdev", "guard|check"]`), `versao_timeout` (10 s).
- `garantir(&Config)` — entrada do host. Síncrona, nunca aborta o host (piso 10): tudo vira linhas no stderr; `panic`
  interno também. **Inerte sob o `cfg(test)` desta lib**; testes do *host* não a executam se ela for chamada do `main`
  (ou proteja com `#[cfg(not(test))]`).
- `garantir_com(&Config, env, args, home: Option<PathBuf>, (os, arch), baixar) -> Option<Resultado>` e
  `garantir_em(&Config, pasta, lock, (os, arch), baixar) -> Resultado` — injetáveis (ambiente, HOME, plataforma e
  downloader), para testes que não tocam o HOME real nem a rede.
- `Resultado { ja_estava, instalados, falhas: Vec<(&str, String)>, avisos, outro_cuida }` — **uma falha por binário**;
  a falha de um não impede o seguinte. `linhas(&Config, &Resultado, pasta) -> Vec<String>` monta as linhas.
- Decisões puras: `deve_rodar`, `eh_hook`, `o_que_falta`, `url_do_asset(nome, os, arch)`, `plataforma_atual()`
  (`aarch64 -> arm64`), `nome_do_arquivo` (`.exe` no Windows), `caminho_novo`.

Comportamento: não roda em `--version/-V/--help/-h`, `help`, `desktop --install|--remover` (só esses; `desktop` sozinho
roda), hooks do host, `CI`, `SCHEMATIZE_SEM_MARKET=1` ou `SCHEMATIZE_DELEGADO_AO_MARKET=1`. Lock
`~/.schematize/run/market-bootstrap.lock` (órfão após 10 min). Fluxo: `curl` -> `<dst>.novo` -> `chmod +x` -> `--version`
(teto 10 s, filho com `SCHEMATIZE_SEM_MARKET=1`, retry de `ETXTBSY`) -> `rename`. Assets: linux/freebsd x86_64,
macos x86_64+arm64, windows x86_64 (`.exe`).

## `procedencia`

`rotulo(versao, sha, sujo) -> String`, `sha_valido(&str) -> Option<&str>`, `sujo_de(&str) -> bool` e o macro:

```rust
// build.rs do app emite as duas variáveis (os NOMES variam por app)
let v: &'static str = schematize_plataforma::rotulo_versao!("SCHEMATIZE_GIT_SHA", "SCHEMATIZE_GIT_DIRTY");
```

O macro expande no app (é o `env!` dele), versão = `CARGO_PKG_VERSION` do app, `OnceLock` para o `&'static str` do clap.

## `bin`

`home() -> Option<PathBuf>` (`HOME` -> `USERPROFILE`; vazio, em branco ou relativo é descartado), `cargo_bin_dir()`,
`resolve_bin(nome) -> Option<PathBuf>` (`~/.cargo/bin` primeiro, depois `PATH`; `.exe/.cmd/.bat/.com` no Windows; exige
bit de execução em unix; nome com `/`, `\` ou vazio -> `None`), `binary_in_path`, e as formas puras `resolve_bin_em`,
`home_de`. Terminal/xdg ficam fora da v0.1.

## `arquivo`

`escreve_atomico(alvo, conteudo) -> Result<(), String>`: temporário único (pid + contador), `sync_all`, `rename`;
remove o temporário se algo falha.

## Desenvolvimento

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo check --target x86_64-unknown-freebsd && cargo check --target x86_64-pc-windows-gnu
```
