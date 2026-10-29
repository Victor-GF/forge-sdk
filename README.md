# forja

Um núcleo de engine **em branco**: sem física, sem renderer pronto. O que ele traz é o
"banco de dados" do projeto. As tabelas são declaradas com layout, tempo de vida e
invariantes, e cada sistema declara o que lê e escreve. A partir dessas declarações a engine:

- monta o grafo de execução e roda em paralelo tudo que não conflita;
- recusa acesso não declarado (um "borrow checker" do frame inteiro);
- desfaz as escritas de um sistema que falhou (escrita transacional), conforme a política declarada;
- mede cada sistema, calcula o caminho crítico e estima o uso de cache line;
- grava relatórios de falha com contexto do scheduler, flight recorder, minidump e replay.

`corrida` é a fatia vertical que valida a ideia: um jogo de corrida top-down com 3 rivais de IA,
obstáculos, crash cinematográfico (câmera lenta, câmera inclinada, letterbox, retícula) e efeitos
estilizados guiados por eventos de gameplay.

Zero dependências externas. A janela Win32 e o GDI usam FFI escrita à mão, e o render é por software.

## O fluxo: editor → arquivo → jogo

```
 forja-editor  ──salva──▶  corrida/projeto.forja  ──lido ao iniciar──▶  corrida (jogo)
      ▲                    (o pipeline: você decide)                         │
      └──────────────lê────  corrida/perfil.forja  ◀──grava ao sair / medir──┘
                           (catálogo de tabelas + tempos + acessos reais)
```

- **`projeto.forja`** é a fonte da verdade do pipeline. Para cada sistema: fase, frequência,
  política de falha, o que lê e escreve (por tabela ou por coluna), nota e se está ativo. É
  texto simples, versionável no git, e também pode ser editado à mão.
- **`perfil.forja`** é gerado pelo jogo e não se edita. Traz as tabelas como existem no código
  (colunas, bytes, linhas), as funções implementadas, o tempo médio de cada sistema e as tabelas
  que cada um *realmente* acessou.
- **O código** registra as funções pelo nome (`systems::register`) e define os structs das
  tabelas (`tables.rs`). Mudar o pipeline não exige recompilar. Mudar o layout de uma tabela exige.
- **A validação é a mesma nos dois lados** (`forja::project::validate`). O editor mostra ao vivo
  exatamente os erros com que o jogo se recusaria a iniciar.

```sh
cargo build --release                                   # compila engine, jogo e editor
cargo run --release -p forja-editor                     # abre corrida/projeto.forja
```

No editor:

| Onde | O que faz |
|---|---|
| Aba **Pipeline** | tabela de sistemas × tabelas. A célula alterna entre não acessa, lê (R) e escreve (W); o botão direito escolhe as colunas (R*). Fase e falha avançam com clique e voltam com o botão direito; a frequência tem − e +; ▲▼ reordena dentro da fase; o quadrado ativa ou desativa o sistema. |
| Aba **Grafo** | o DAG que sai das células, com o caminho crítico em amarelo e a folga de cada sistema |
| Aba **Tabelas** | catálogo (layout, bytes, padding, uso de cache); ao clicar numa tabela, mostra quem usa cada coluna (quentes x frias) |
| Painel inferior | detalhes do selecionado: de quem depende e por quais tabelas, custo da transação, uso de cache por acesso, nota editável |
| Painel direito | validação ao vivo e sugestões da análise |
| **F6 Medir** | roda o jogo headless (1800 ticks) com o pipeline atual e atualiza tempos e acessos reais |
| **F5 Rodar** | salva e abre o jogo com este pipeline (recusa se houver erro) |
| Ctrl+S / Ctrl+Z / Ctrl+Y / Ctrl+R | salvar / desfazer / refazer / recarregar do disco |

Um ponto vermelho numa célula significa que o sistema declara a tabela mas não a acessou na última
medição: declaração a mais.

`forja-editor --shot saida.png [--aba grafo|tabelas] [--sel Sistema] [--cols Sistema:Tabela]` desenha
um frame direto num PNG, sem abrir janela.

## Rodar o jogo

```sh
cargo run --release -p corrida                          # jogar
cargo run --release -p corrida -- --autopilot           # o carro dirige sozinho
cargo run --release -p corrida -- --replay <input.log>  # reproduz uma partida/crash gravado
cargo run --release -p corrida -- --analyze 3600        # headless: gera reports/analysis.md
```

Opções do modo headless (úteis para testes e CI):

| Opção | Efeito |
|---|---|
| `--inject 300:nan` | injeta uma falha no tick 300 (`nan`, `vfx`, `rules`, `av`, `hang`) |
| `--force-crash 1200` | dispara um evento de crash do jogador (testa a cinemática) |
| `--shot 1212` | grava `reports/shot_1212.png` (framebuffer + HUD + editor) |
| `--shot-view timeline` | visão do editor na captura (`timeline`, `matriz`, `tabelas`, `falhas`) |
| `--workers 3` | número de workers do job pool |

## Teclas

| Jogo | | Editor (overlay) | | Falhas injetadas | |
|---|---|---|---|---|---|
| Setas | dirigir | F1 | timeline do frame | F5 | NaN na física: invariante + rollback |
| Espaço | freio de mão (drift) | F2 | matriz sistemas x tabelas | F6 | panic no VFX: rollback + desliga |
| Shift | boost | F3 | tabelas (layout, bytes, cache) | F7 | panic nas regras: crash + minidump |
| T | piloto automático | F4 | falhas, relatórios, eventos | F8 | access violation (SEH) |
| L | salvar replay | P / W | congelar / pior frame | F9 | trava 6 s (watchdog) |
| Esc | sair | I / R | invariantes / religar sistemas | | |

As falhas injetadas entram no log de input. Por isso o `--replay` do `input.log` de um relatório
reproduz o mesmo bug no mesmo tick.

## Estrutura

```
forja/            engine genérica (não sabe nada de corrida)
  world.rs        tabelas: colunas, layout SoA/AoS, tempo de vida, invariantes, snapshot/restore
  schedule.rs     SystemDecl (reads/writes/fase/frequência/política), grafo, execução, SysCtx
  jobs.rs         job pool com scopes (empréstimos da pilha, sem threads por frame)
  profiler.rs     zonas por sistema/lane + flight recorder (últimos 600 frames)
  analysis.rs     modelo puro (TableModel/GraphModel): caminho crítico, folga, cache, sugestões
  project.rs      projeto.forja / perfil.forja: parser, escrita, validação, montagem do grafo
  editor.rs       overlays F1–F4 dentro do jogo
  crash.rs        panic hook, filtro SEH + minidump, watchdog, relatórios, log de input
  platform.rs     Win32/GDI à mão (janela, teclado, mouse, texto)
  draw.rs         rasterização por software em faixas paralelas
  png.rs          capturas de tela sem dependências
editor/           forja-editor: o pipeline como tabela (abas Pipeline, Grafo, Tabelas)
corrida/          o jogo
  projeto.forja   o pipeline (editado no forja-editor)
  perfil.forja    gerado pelo jogo
  tables.rs       o banco de dados do jogo (agrupado por tempo de vida) + invariantes
  systems.rs      13 funções de sistema, registradas pelo nome
  render.rs       3 sistemas de render (cenário, carros "hero", VFX/pós)
  track.rs        cook da pista (spline) e do cenário assado
```

### Como um sistema é declarado

No `projeto.forja` (ou pelo editor):

```ini
[sistema Vehicle.Physics]
fase = Fisica
a_cada = 1
falha = pular_tick
ativo = sim
le = VehicleControl Track CrashState(mode) DebugFaults
escreve = VehicleHot VehicleSim Events
nota = 4 substeps (240 Hz)
```

No código, só a função e o nome:

```rust
fns.push(("Vehicle.Physics", vehicle_physics));
```

A função `vehicle_physics` é escrita de forma monolítica, do jeito que se pensa. O grafo, o
paralelismo, as transações e a análise saem da declaração. Se a função tentar acessar uma tabela
que o arquivo não declara, o acesso falha na hora. Se o arquivo declarar uma tabela que a função
nunca usa, o editor mostra depois da próxima medição.

(A API em código, `SystemDecl::new(...).reads::<T>()...`, continua existindo na engine;
`Project::from_schedules` exporta declarações em código para o formato de arquivo.)

### Políticas de falha

| Política (no arquivo) | O que acontece | Usada em |
|---|---|---|
| `abortar` | relatório + minidump, encerra | Input, regras de corrida, colisão |
| `pular_tick` | rollback das tabelas escritas, roda de novo no próximo tick | física |
| `desligar` | rollback + desliga até religar (R) | IA, Diretor, VFX |
| `desligar_sem_rollback` | desliga sem copiar nada | render (a saída é refeita a cada frame) |

## O que a análise já mostrou neste projeto

Do `reports/analysis.md` gerado pelo `--analyze`:

- **O render domina; a simulação é barata.** O grafo `tick` tem caminho crítico de ~0,03 ms e o `frame`, ~2,7 ms.
- **O custo do scheduler é maior que o trabalho da simulação.** São ~0,07 ms de parede por tick para 0,03 ms de trabalho. Com 4 carros, os sistemas são pequenos demais para o custo de despacho de um job cada; agrupar sistemas ou paralelizar por entidade só compensa com mais dados.
- **`Events` tem 4 escritores** e serializa física, colisão e regras. Separar por produtor libera paralelismo.
- **Layout:** a física lê 1 B de cada linha de 40 B de `CrashState` (2%), e a IA usa 20% de `Obstacles`. São candidatas a separação entre dados quentes e frios.
- **Política de falha errada pega pela análise:** com `Disable`, o render copiava o framebuffer (3,5 MB) duas vezes por frame só para poder fazer rollback. Trocar por `DisableNoRollback` levou o grafo `frame` de 4,0 ms para 2,7 ms.

## Kit de falhas

Cada relatório fica em `reports/<data>_<tipo>_<título>/`:

- `report.txt`: o que cada lane executava, sistemas desligados, falhas anteriores, breadcrumbs, panic com backtrace ou exceção nativa com endereço;
- `flight_recorder.json`: últimos ~6 s de profiling (abra em https://ui.perfetto.dev);
- `input.log`: todos os inputs desde o tick 0, mais checkpoints de hash do estado a 1 Hz;
- `dump.dmp`: minidump para crashes, abortos e travamentos (Visual Studio ou WinDbg; os `.pdb` ficam em `target/release`).

O replay confere os checkpoints de hash e avisa no HUD se a simulação divergir. Isso funciona como
teste de determinismo de graça.

## Limitações conhecidas e próximos passos

- **O layout das tabelas continua em código** (`tables.rs`). O editor mostra o catálogo, mas mudar
  colunas, separar dados quentes de frios ou trocar SoA por AoS exige editar os structs e recompilar.
  O próximo passo é gerar os structs a partir de um arquivo de esquema.
- **Os tempos no editor vêm da última medição.** Se você mudar o pipeline, o caminho crítico mostrado
  usa os tempos antigos até rodar Medir (F6) de novo.
- **Pipelining entre simulação e render:** o render já lê só o `RenderSnapshot` imutável, então pode rodar
  em paralelo com o tick seguinte. Ainda não roda.
- O editor não cria sistemas novos: um sistema precisa existir como função no código. Funções
  implementadas que estão fora do pipeline aparecem como aviso.
- Só Windows (FFI Win32). A engine em si é portátil; a plataforma e o kit de crash não são.
- Renderização por software em 1280x720. Uma GPU entraria como mais um backend do grafo `frame`.
