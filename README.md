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
| `jaula` | `jaula` — teto duro de memória por execução; o OOM mata só a execução | não |
| `sistema` | `sistema` — detecta o ambiente de memória/OOM do host (cgroup v2, PSI, oomd, zram, systemd). **Só lê** | não |
| `memoria` | `memoria` (implica `sistema`) — monitor de cgroup/PSI, governador de admissão, histórico de picos | não |
| `persistencia` | `persistencia` + bin `schematize-vigia` — journal WAL, snapshot atômico, máquina idempotente, vigia | não |

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

## `jaula` (feature `jaula`, v0.3.0)

Teto duro de memória **por execução**: o OOM mata só a jaula, nunca o orquestrador. Backend
`systemd-run --user --scope` (cgroup v2, `memory` delegado; `OOMPolicy=kill`, `MemorySwapMax=0`,
`OOMScoreAdjust` maior que o do pai); fallback `prlimit`; sem nenhum, `Veredito::SemBackend` (não roda).

```rust
use schematize_plataforma::jaula::{estimar, executar, TipoExecucao, Veredito};
let argv = vec!["cargo".to_string(), "test".to_string()];
match executar(&estimar(TipoExecucao::Teste), &argv, "teste-1") {
    Veredito::EstourouMemoria { pico_mib, teto_mib } => eprintln!("OOM: {pico_mib:?}/{teto_mib}"),
    outro => eprintln!("{outro:?}"),
}
```

Nota: o scope do systemd não aceita `OOMScoreAdjust=`; o score entra por um wrapper `sh` com argv puro. Sem swap, memória anônima presa no `MemoryHigh` por >3 s mata a unidade (o kernel só faria throttle).

`Orcamento::novo` clampa em [500, 2048] MiB (acima só com `override_adr`); `estimar` mapeia
`AgenteLlm/Teste→1024`, `Script/Tool→512`, `Build/Navegador→2048`; `jobs_sugeridos` ≈ teto/700 MiB.

## Desenvolvimento

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo check --target x86_64-unknown-freebsd && cargo check --target x86_64-pc-windows-gnu
```

## `sistema` (feature `sistema`)

Detector **somente-leitura** do que o host faz com memória: versão do kernel, cgroup v2, `memory.*`, PSI, daemons de
OOM (systemd-oomd, earlyoom...), zram/swap, e uma lista de **achados** (problemas) com `margem_segura`.
Garantias: só lê `/proc` e `/sys` (e a saída de comandos de consulta); **nada é instalado, alterado ou escrito**;
ausência de qualquer fonte vira "desconhecido", nunca erro nem pânico. Limites: Linux (cgroup v2); em outros SO devolve vazio.

## `memoria` (feature `memoria`)

Controle fino em runtime sobre o `sistema`: leitura de PSI e `memory.events`, monitor de cgroup, **governador** que decide
admissão de trabalho por orçamento adaptativo, e histórico de picos (anel em memória). Garantias: decisões puras e
testadas com fixtures; só lê. Limites: não impõe teto (para isso a `jaula`); depende de cgroup v2 para os sinais finos.

## `persistencia` (feature `persistencia`) e `schematize-vigia`

Para o orquestrador retomar exatamente de onde parou depois de SIGKILL/OOM/queda de energia:

- `Journal`: WAL append-only `[len][crc32][seq][tipo][payload]`, segmentos rotacionados, fsync configurável
  (`Durabilidade::Cada` é o default e garante "Ok == sobrevive à queda"), escritor único por `flock`, recuperação que
  trunca a cauda torn e **recusa** buraco de seq/corrupção no meio.
- `Snapshot`: atômico (tmp + fsync + rename + fsync do diretório), versionado, com `.prev`.
- `Maquina`/`Estado`: `estado = snapshot + replay`, eventos idempotentes por seq.
- `rodar(&VigiaCfg)` e `unit_systemd(&VigiaCfg)`: o vigia e a unit de usuário do systemd (texto; **nada é instalado**).

### Binário `schematize-vigia`

```text
schematize-vigia --estado <dir> [--max-mortes N] [--janela-s S] [--backoff-ms B] -- <cmd> [args...]
schematize-vigia --estado <dir> --unit-systemd -- <cmd> [args...]   # imprime a unit, nada é instalado
schematize-vigia --estado <dir> --status                            # mortes, parked, última causa
schematize-vigia --estado <dir> --limpar-parked --sim               # reabre o circuito (exige --sim)
schematize-vigia --help | --version
```

Compilar: `cargo build --release --features persistencia` (o bin exige só essa feature; sem deps). Relança `<cmd>` com
`--resume` quando morre (backoff exponencial + jitter) e abre o circuito (`parked`) após N mortes na janela (default 5 em
600 s). Mensagens em pt/en (`SCHEMATIZE_LANG`, `LC_ALL` ou `LANG`).

| exit | significado |
|---|---|
| 0 | concluiu (filho saiu 0) ou ação auxiliar ok |
| 1 | erro de execução/E-S |
| 2 | uso incorreto |
| 3 | parked (agora ou de antes; também no `--status`) |

Garantias: RSS ocioso ~2,7 MiB (medido, ver `BUDGET.md`); journal próprio fsync a cada registro; um só vigia por estado.
Limites conhecidos: só Unix (flock, `/proc`); `oom_score_adj` negativo exige `CAP_SYS_RESOURCE` (sem ele é ignorado e
registrado como `oom_adj_aplicado=false`); o torn-write natural (queda de energia no meio de um registro) nunca foi
observado, só simulado por truncagem; falha de `fsync` **envenena** o journal (`Erro::Envenenado`: reabra e recupere).
