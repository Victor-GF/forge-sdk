# forja

Núcleo de engine de jogos orientado a dados, em Rust, sem dependências além de `windows-sys`
(só para abrir a janela). Foco técnico/sistêmico: layout de dados eficiente, multithreading
seguro por construção, frame com fases bem definidas, e física e sombreamento escritos com as
mesmas ferramentas que o jogo usa.

## Piso de hardware

Em x86-64 o projeto compila para **x86-64-v3**: AVX2, FMA, BMI1/2, F16C, LZCNT, MOVBE
(Intel Haswell 2013+, AMD Excavator/Zen 2015+). Isso está em `.cargo/config.toml`.
`JobSystem::new` chama `cpu::check_floor()`, que consulta a CPU via `cpuid`/`xgetbv` e encerra com
uma mensagem clara (em vez de "illegal instruction") se faltar alguma extensão.

Sem o flag (outra arquitetura, ou um crate que use o forja como dependência sem configurar), o
código cai no caminho SIMD portátil: mesma API, resultado equivalente (difere só no arredondamento,
porque o FMA faz multiplicação e soma com um único arredondamento), mais lento. Para testar esse
caminho nesta máquina:

```bash
CARGO_TARGET_DIR=target/portable RUSTFLAGS="-C target-cpu=x86-64" cargo test --release
```

## Como rodar

```bash
cargo test --release
```

```bash
cargo run --release --example rain
```

```bash
cargo run --release --example rain -- --headless 600 --threads 8 --balls 4000
```

```bash
cargo run --release --example bench
```

```bash
cargo run --release --example sandbox
```

O modo `--headless` roda determinístico (1 tick por frame), salva `out/rain.png` e
`out/trace.json` (abra em <https://ui.perfetto.dev> para ver a linha do tempo por thread).
Na janela: setas/WASD orbitam, Q/E zoom, Espaço +1000 bolas, R liga/desliga rotação,
P grava o trace, Esc sai.

### Sandbox

Cena de testes onde cada recurso de física é usado por um sistema de jogo
([examples/sandbox.rs](examples/sandbox.rs)):

| Zona | O que exercita |
|---|---|
| Carro | Veículo por raycast: chassi rígido, suspensão e pneus a partir de 4 raios; canhão no teto com recuo |
| Rampa de salto | Caixa estática inclinada; o carro decola e aterrissa nas próprias suspensões |
| Cortina de cordas | 8 correntes de partículas com hastes + *tethers*; o carro atravessa empurrando-as |
| Pirâmide + bola de demolição | 15 caixas rígidas empilhadas; bola de 900 kg numa corrente de 18 elos |
| Trampolim | Rede 22×22 de partículas com molas (XPBD), borda presa, sob uma fonte de bolas |
| Poço de bolinhas | 900 esferas num cercado; dá para entrar com o carro pela rampa |
| Piscina | Empuxo e arrasto: bolas e caixas flutuam, o carro também |
| Granada | Raycast do centro da tela escolhe o ponto, impulso radial |

Teclas: W/S acelera/ré, A/D esterça, Espaço freio de mão, F atira, G granada, B balança a bola de
demolição, R desvira o carro, N +300 bolas, C alterna câmeras (perseguição e uma fixa por zona),
setas orbitam, Q/E zoom, P grava o trace.

No modo headless um piloto automático dirige, atira e detona granadas:

```bash
cargo run --release --example sandbox -- --headless 600 --view 2 --shots 60
```

`--view N` fixa a câmera numa zona (1 = visão geral, 2 = pirâmide, 3 = cordas, 4 = trampolim,
5 = piscina) e `--shots 60` salva um PNG por segundo em `out/`.

## Camadas

| Módulo | O que é |
|---|---|
| `jobs` | Pool fixo de threads. `scope` (jobs que emprestam da pilha), `par_for` / `par_zip` (laços paralelos sobre fatias SoA), `spawn` (jobs de fundo). |
| `world` | Entidades em tabelas SoA, recursos globais, `Commands` para mudanças estruturais. |
| `schedule` | Sistemas = funções + declaração de leitura/escrita. O grafo de dependências é derivado, nunca escrito à mão. |
| `frame` | O modelo de frame: relógio de passo fixo, ponto de extração, render em pipeline. |
| `arena` | Alocador linear por frame (reset em O(1), zero `free`). |
| `simd` | `F32x8` / `I32x8` / `M32x8`: 8 lanes, implementação AVX2+FMA ou portátil com a mesma API. |
| `cpu` | Verificação do piso de hardware em tempo de execução. |
| `physics` | PBD em substeps: milhares de esferas (Jacobi paralelo, SIMD), caixas rígidas, restrições de distância, caixas estáticas, raycast e explosões. Em cima disso, só com a API pública: veículo por raycast e água. |
| `render` | Rasterizador por software: culling, LOD, binning em faixas, shadow map, Blinn-Phong. |
| `profile` | Escopos nomeados por thread, relatório de utilização e export para Perfetto. |
| `alloc` | Alocador global opcional que conta alocações, bytes vivos e pico. |
| `telemetry` | Trilha de frames, contadores de memória e o resumo de escala, cache e threads. |
| `platform` | Janela Win32 + teclado. O núcleo não depende dela. |

## O conceito de frame

Um frame é uma sequência fixa de fases, e cada fase tem **um dono claro dos dados**:

```text
            thread principal: &mut World                 |  job de render: (Renderer, Snapshot) por valor
frame N:    [begin] [tick]*k [extract N] ──entrega──▶ [render N] ..............................
frame N+1:                         [begin] [tick]*k [extract N+1] ─espera N─▶ [render N+1] ...
```

- **begin** (dt variável): input, câmera, tudo que acompanha a taxa de exibição.
- **tick** (dt fixo, 0..`max_ticks` por frame): simulação. Passo fixo = física estável e
  replays determinísticos. O `FrameClock` acumula tempo real e diz quantos ticks rodar; se a
  máquina não aguenta, a simulação desacelera em vez de entrar em espiral.
- **extract**: o *único* ponto de sincronização entre simulação e render. Copia o que o
  renderer precisa para um `Snapshot` (dado puro, com dono).
- **render**: roda como job de fundo **enquanto o próximo frame simula**. Ele é dono do renderer
  e do snapshot por valor — não há estado mutável compartilhado nem locks. A posse volta para a
  thread principal na extração seguinte (ping-pong de dois snapshots, zero alocação em regime).

Todas as fases usam o mesmo pool. Os lotes do render preenchem os buracos do grafo da
simulação e vice-versa: a CPU fica ocupada em vez de parar numa barreira. Quem espera um `scope`
ajuda, mas **só com os jobs daquele escopo** (isolamento de tarefas, como o `isolate` do TBB);
workers ociosos pegam qualquer job. Sem isso, a thread principal, esperando um lote de 50 µs da
física, pegava uma faixa de 5 ms do render do frame anterior e só voltava quando ela acabasse — a
telemetria mostrou ~19 ms/frame de render rodando dentro dos passes da física.

## Sistemas: monólito ou tasks encadeadas?

Nenhum dos dois puro. A regra aqui é:

1. **Sistemas grossos, com acesso declarado** (paralelismo de tarefas). Um sistema é uma função
   comum que diz o que lê e escreve:

   ```rust
   system("heat", heat)
       .reads::<PhysicsWorld>().reads::<Body>().reads::<Tint>()
       .writes::<Renderable>()
   ```

   A ordem de inserção é a ordem lógica. O `Schedule` cria uma aresta A → B só quando há
   conflito (um escreve o que o outro lê ou escreve) ou `.after("A")`. O resto roda junto.
   `schedule.describe()` mostra as *waves*:

   ```text
   wave 1  physics.step   writes [PhysicsWorld]
   wave 2  physics.sync   reads [PhysicsWorld, Body] writes [Transform]
   wave 2  heat           reads [PhysicsWorld, Body, Tint] writes [Renderable]
   ```

2. **Lotes dentro do sistema** (paralelismo de dados). Dentro de um sistema você pega colunas
   inteiras como fatias e divide com `par_zip`:

   ```rust
   fn heat(ctx: &SystemCtx) {
       let phys = ctx.res::<PhysicsWorld>();
       for table in ctx.query::<(Body, Tint, Renderable)>() {
           let mut render = table.write::<Renderable>();
           let (bodies, tints) = (table.read::<Body>(), table.read::<Tint>());
           ctx.jobs.par_zip((&mut *render, &*bodies, &*tints), 1024, |_, (r, b, t)| {
               for i in 0..r.len() {
                   let speed = phys.vel[b[i].0.0 as usize].length();
                   r[i].color = t[i].0.lerp(HOT, (speed / 12.0).min(1.0));
               }
           });
       }
   }
   ```

Por que assim: micro-tasks encadeadas à mão viram um grafo frágil e pagam overhead por task;
monólitos não escalam com núcleos. Sistemas do tamanho de "uma transformação de dados", com o
grafo calculado, e laços em lote por dentro, escalam e continuam legíveis. Granularidade de lote:
algumas centenas a poucos milhares de elementos (o suficiente para amortizar o despacho e caber
em L1/L2).

## Layout de dados

- Cada **conjunto distinto de componentes** é uma `Table`; cada componente é um `Vec<T>`
  (structure of arrays). A linha `i` de todas as colunas é a entidade `entities[i]`.
- Sistemas nunca veem entidades uma a uma: emprestam colunas como `&[T]` / `&mut [T]` e fazem
  laços simples — o que o cache e o vetorizador querem.
- Não há "adicionar componente" em tempo de execução: trocar de conjunto = `despawn` + `spawn`
  via `Commands`. Sem migrações escondidas.
- Subsistemas grandes (física, render) têm armazenamento próprio em SoA, como Jolt/PhysX; a
  entidade só aponta (`Body(BodyId)`) e um sistema de sync copia o resultado.
- Memória transitória vem da `FrameArena` (o render aloca vértices, triângulos e listas nela).

## Segurança no multithreading

- Durante a execução de sistemas o mundo é `&World`. Mudanças estruturais exigem `&mut World`,
  então só acontecem nos pontos de sincronização (aplicação de `Commands`, em ordem de sistema).
- Cada coluna e recurso fica num `SyncCell`: um lock leitura/escrita atômico que **entra em
  pânico em vez de bloquear**. O scheduler ordena pelo acesso declarado, então o conflito não
  acontece na prática; o cell transforma uma declaração errada num pânico com mensagem clara, não
  numa data race.
- O `SystemCtx` checa a declaração: acessar algo não declarado dá pânico imediato
  (`system 'x' accessed 'T' without declaring .writes::<T>()`), mesmo sem concorrência.
- `par_zip` só entrega fatias disjuntas; o `unsafe` fica concentrado em `jobs.rs` (apagar o
  tempo de vida de jobs com escopo e dividir fatias) e `arena.rs`, com a justificativa ao lado.
- Determinismo por padrão: a física e a imagem são idênticas bit a bit com 1, 3 ou 8 threads
  (há testes para isso).

## SIMD

`simd::F32x8` é um tipo de valor com operadores, comparações que geram máscara (`M32x8`),
`select`, `mul_add` (FMA), `gather` com checagem de limites, `exp` vetorizado e `powi`. Os laços
quentes processam 8 elementos por passo em forma de structure-of-arrays:

- **rasterizador**: 8 pixels de uma linha por passo — funções de aresta, máscara de cobertura e
  teste de profundidade mascarado; blocos alinhados em múltiplos de 8;
- **sombreamento**: 8 pixels por vez; cada lane busca (gather) o seu triângulo, vértices e cor da
  instância, e toda a iluminação (Blinn-Phong, PCF 3×3, névoa com `exp`) roda vetorizada;
- **física**: cada corpo testa seus vizinhos 8 por vez (gather de posição, raio e massa).

`Vert`, `Tri`, `Instance` e `Vec3` são `repr(C)` só com campos de 4 bytes, então uma fatia deles
pode ser lida como palavras pelos gathers. O `unsafe` fica em `simd/avx2.rs` (intrínsecos e
loads/stores com tamanho checado) e nas conversões de fatia, com a justificativa ao lado. Os
testes comparam a implementação AVX2 com a portátil operação por operação.

Lição registrada na física: vetorizar só a conta de colisão deixou o passo **mais lento**, porque o
custo estava na busca (27 consultas ao hash por corpo, por iteração). O ganho veio de mudar o
dado — lista de vizinhos por corpo montada uma vez por substep — e só então vetorizar sobre ela.

## Física

Position-Based Dynamics em substeps (padrão: 4 substeps × 2 iterações), dentro de uma caixa. Tudo
mora em `PhysicsWorld`, em arrays próprios; as entidades apontam para os corpos (`Body`, `Rigid`) e
`physics.sync` copia o resultado para o `Transform`.

| Peça | Como funciona | Paralelismo |
|---|---|---|
| Partículas (esferas) | Hash espacial com counting sort, lista de vizinhos por corpo, contatos Jacobi: cada corpo *coleta* correções (8 por vez, SIMD) e escreve só o próprio slot | `par_zip` sobre os corpos |
| Grupos | Partículas do mesmo grupo não colidem entre si (elos de uma corda, nós de uma rede) | — |
| Restrições de distância | Hastes, molas (XPBD, com *compliance*) e *tethers* (só puxam; long-range attachments de Kim et al. 2012, impedem a corrente de esticar sob carga) | Gauss-Seidel serial: milhares custam microssegundos |
| Caixas rígidas | XPBD com orientação (Müller et al. 2020). Colisão por 26 pontos amostrados (cantos, meios de aresta, centros de face) contra chão, paredes, caixas estáticas e outras caixas; partícula contra caixa é exata | Serial, ordem fixa (dezenas de corpos) |
| Atrito das caixas | No nível de velocidade, limitado por μ × impulso normal | Serial |
| Estáticos | Caixas orientadas que nunca se movem: rampas, paredes, postes | Dentro do passe paralelo das partículas |
| Consultas | `raycast` (estáticos, caixas e partículas, estas 8 por vez com SIMD) e `explode` (impulso radial) | — |

Mecanismos de jogo escritos só com essa API pública, como um sistema do jogo seria:

- **`vehicle::RaycastVehicle`**: o modelo de carro da maioria dos jogos de ação (`btRaycastVehicle`,
  WheelCollider). Cada roda é um raio; o acerto vira mola-amortecedor e forças de pneu (atrito
  lateral, motor, freio, círculo de atrito), aplicadas no chassi. A reação vai para o que a roda
  pisa, então passar por cima de bolas as empurra para o chão. `roll_influence` controla o quanto o
  carro inclina nas curvas.
- **`water::WaterVolume`**: empuxo de Arquimedes (volume submerso da calota esférica) e arrasto.
  Para caixas, forças em 27 pontos, o que gera torque de endireitamento.

Duas lições registradas no código:

- Escolher a face de saída "mais próxima" por ponto derruba pilhas alinhadas: os cantos da caixa de
  cima ficam exatamente sobre as faces laterais da de baixo e são empurrados de lado. A face é
  escolhida uma vez por par, pela direção entre os centros.
- O atrito estático no nível de posição (como no artigo) fazia caixas em repouso deslizarem: cada
  correção normal inclina a caixa um pouco, o atrito desfaz isso transladando a caixa inteira, e a
  ordem sequencial nunca fecha a conta. Numa pilha de 2 caixas, em 10 s: 7 cm de deriva com ele,
  0,2 mm sem.

## Sombreamento

Rasterizador por software, tudo paralelo no pool:

1. **instâncias**: frustum culling por esfera envolvente e LOD pelo raio projetado em pixels;
2. **geometria**: transforma cada vértice uma vez, monta e descarta triângulos (backface,
   pequenos demais, fora da tela), saída na arena. Instâncias cuja esfera envolvente cruza o plano
   near reservam espaço extra e seus triângulos são recortados contra ele (Sutherland-Hodgman);
   as outras não pagam nada por isso;
3. **binning**: counting sort em faixas horizontais, histograma por bloco, sem contenção;
4. **shadow map**: mesmo rasterizador visto do sol, só profundidade, só faces traseiras;
5. **passada principal**: por faixa, *visibility buffer* (profundidade + id do triângulo) e
   depois sombreamento de cada pixel **uma única vez**: interpolação com correção de
   perspectiva, Lambert + Blinn-Phong, ambiente hemisférico, sombra PCF 3×3, névoa.

Uma faixa são `strip_height` linhas inteiras, então os pixels dela são uma fatia contígua do
framebuffer: cada thread recebe a sua com `par_zip`.

## Telemetria: memória, threads, espera e cache

```bash
cargo run --release --example sandbox -- --telemetry 120
```

Roda a mesma cena roteirizada com 1, 2, 4 e 8 threads. Em cada uma, aquece por 240 frames e grava 120.
Rode com a máquina livre (sem build, sem navegador pesado): com a CPU disputada, passes paralelos
esperam por threads que o SO tirou de cena e a curva de escala sai errada.

Cada fatia do trace tem um **nível**, e o relatório desce do macro ao micro:

| Nível | O que é | Quem registra |
|---|---|---|
| `phase` | etapa do frame: `extract`, `wait render`, `present` | `FramePipeline` |
| `schedule` | uma execução de schedule (`frame`, `tick`) | `Schedule::run` |
| `system` | um sistema | executor do schedule |
| `pass` | trabalho nomeado dentro de um sistema (`physics.contacts`) | `profile::scope` / `scope_bytes` |
| `job` | um job tirado da fila, com o nome de quem o criou | `jobs` |
| `wait` / `idle` | esperando (ajudando quando pode) / dormindo sem trabalho | `jobs` |

Toda fatia também leva o **frame de origem**: o render do frame N roda durante o frame N+1 e
aparece como trabalho do frame N. Cada job leva **quanto tempo esperou na fila** (`queued_us`).
E cada `spawn` desenha uma **seta** de quem criou o job até quem o executou (e do fim de um job
até quem esperou por ele). Assim, o grafo de dependências dos sistemas, a distribuição de um
`par_for` e a passagem simulação → render aparecem no tempo, sem código extra.

Saídas:

- **Terminal**, de cima para baixo:
  - *macro*: por número de threads, tempo de frame, latência (do início da simulação até a imagem
    ser apresentada), simulação, render, sobreposição entre os dois e quantas threads trabalham,
    esperam e dormem;
  - *meso*: cada sistema do tick, com quando começa, quanto dura e em que thread roda;
  - *micro*: passes (escala 1T→8T, footprint, cache, GB/s) e jobs por origem (quantos, duração,
    espera na fila média e p95, quantos a própria main executou);
  - threads e memória.
- **`out/telemetry.json`:** abra em <https://ui.perfetto.dev> (Open trace file). Cada número de
  threads é um processo; dentro dele:
  - no topo, as trilhas do pipeline: `1 pipeline: simulate`, `2 pipeline: render`,
    `3 pipeline: present` e a latência de cada frame (em duas trilhas, porque frames vizinhos se
    sobrepõem);
  - a trilha `frames`: uma fatia por frame, com alocações, KiB alocados, heap vivo e jobs nos args;
  - uma trilha por thread, com as fatias categorizadas pelo nível (filtre por categoria);
  - contadores por frame: threads trabalhando, esperando e ociosas, espera média na fila,
    latência, tempo de frame, heap vivo, alocações, arena do render e `mem: <subsistema>`;
  - clique numa fatia para ver as setas e os args (`frame`, `queued_us`, `bytes`, `KiB`, `GBps`,
    `allocs`).

De onde vêm os dados:

| Medida | Fonte |
|---|---|
| Tempo, espera, ociosidade | `profile`: escopos por thread com nível; `(wait)` no `help_while`, `(idle)` no sono do worker |
| Pipeline e latência | `telemetry::frame_lives`: agrupa as fatias pelo frame de origem |
| Distribuição dos jobs | `jobs`: frame de origem, espera na fila e seta de quem criou cada job |
| Alocações | `alloc::CountingAlloc` (o binário instala como `#[global_allocator]`); cada escopo guarda as alocações feitas na própria thread enquanto estava aberto |
| Memória por subsistema | `World::memory`, `PhysicsWorld::memory`, `SoftwareRenderer::memory`: capacidade reservada dos arrays |
| Footprint | Declarado por cada passe com `profile::scope_bytes` (quem conhece o layout calcula os bytes) |
| Caches | `cpu::caches()`: `cpuid` folha 4 (Intel) ou 0x8000001D (AMD) |

O footprint é uma estimativa feita a partir do layout dos dados, não uma medição de falhas de cache.
Para contar falhas de verdade no Windows é preciso um profiler com acesso aos contadores da CPU
(Intel VTune, gratuito; ou o Windows Performance Recorder como administrador).

Consultas úteis no Perfetto (aba Query (SQL)):

```sql
-- tempo total por passe e por execução (inclui o tempo dos escopos internos)
SELECT p.name AS run, s.name, ROUND(SUM(s.dur) / 1e6 / 120, 2) AS ms_per_frame, COUNT(*) AS calls
FROM slice s JOIN thread_track tt ON s.track_id = tt.id JOIN thread USING (utid) JOIN process p USING (upid)
GROUP BY run, s.name ORDER BY run, ms_per_frame DESC;
```

```sql
-- footprint e banda por passe
SELECT name, MAX(extract_arg(arg_set_id, 'args.KiB')) AS kib, ROUND(AVG(extract_arg(arg_set_id, 'args.GBps')), 2) AS gbps
FROM slice WHERE extract_arg(arg_set_id, 'args.bytes') IS NOT NULL GROUP BY name ORDER BY kib DESC;
```

```sql
-- espera na fila por origem dos jobs
SELECT p.name AS run, s.name, COUNT(*) AS jobs, ROUND(AVG(extract_arg(s.arg_set_id, 'args.queued_us')), 1) AS fila_us
FROM slice s JOIN thread_track tt ON s.track_id = tt.id JOIN thread USING (utid) JOIN process p USING (upid)
WHERE s.category = 'job' GROUP BY 1, 2 ORDER BY 1, fila_us DESC;
```

```sql
-- espera e ociosidade por thread
SELECT p.name AS run, t.name AS thread, s.name, ROUND(SUM(s.dur) / 1e6, 1) AS ms
FROM slice s JOIN thread_track tt ON s.track_id = tt.id JOIN thread t USING (utid) JOIN process p USING (upid)
WHERE s.name IN ('(wait)', '(idle)') GROUP BY 1, 2, 3 ORDER BY 1, 2;
```

### O que a telemetria já mostrou

Primeira correção guiada por ela: isolar a espera por escopo. A/B no mesmo momento, sandbox com
8 threads:

| | ajuda com qualquer job | só com o próprio escopo |
|---|---|---|
| frame (duas rodadas) | 9,4–10,5 ms | **8,0–8,3 ms** |
| simulação (com o render em paralelo) | 10,4 ms | **5,1 ms** |
| `physics.contacts` | 4,0 ms | 0,7 ms |
| workers ocupados | 74% | 87% |

Com 1 thread, ou sem render (`--no-render`), nada muda, como esperado.

Pontos que continuam abertos:

- A simulação leva 2,8 ms sozinha e 5,1 ms com o render em paralelo: núcleos com hyperthreading e
  o L3 são divididos. O render passa ~9 MiB por frame, mais que os 8 MiB do L3.
- Trechos seriais da física: a counting sort do broadphase, as caixas e as restrições. Com a física
  sozinha, o ganho de 1 para 8 threads é só 1,7x.
- ~190 alocações por frame, principalmente o `Box` de cada job de `par_for`.
- A arena do render reserva 32 MiB e usa no pico 8,2 MiB.
- O frame dura o mesmo que o render (8,8 ms com 8 threads); a simulação (6,2 ms) roda por baixo.
  A latência é de dois frames (17,6 ms): a imagem do frame N só é apresentada no submit do N+1.
- Os ajudantes da física esperam ~0,3 ms na fila (p95 ~0,9 ms) atrás de jobs do render que duram
  2–3 ms; por isso a main acaba fazendo o trabalho deles e a simulação piora com mais threads.

## Números (i7-7700, 4 núcleos / 8 threads)

Benchmark fixo (`--example bench`: 3000 esferas assentadas, mediana de 200 passos / 100 frames):

| | física (passo) | render (frame 960×540 + sombra 1024²) |
|---|---|---|
| escalar, x86-64 base | 4,20 ms | 20,2 ms |
| só o flag x86-64-v3 (auto-vetorização) | 4,04 ms | 18,0 ms |
| SIMD explícito + lista de vizinhos | **2,7–2,9 ms** | **12,7–14,3 ms** |
| caminho portátil (sem o flag) | 3,2 ms | 21,9 ms |

Exemplo `rain` completo (simulação + render em pipeline): ~95 ms/frame na primeira versão →
~25 ms com LOD, culling e binning sem contenção → **~15 ms** com SIMD.

## Limitações conhecidas / próximos passos

- Geometria (transformação de vértices e montagem de triângulos) ainda é escalar; é a próxima
  etapa a vetorizar.
- A montagem das listas de vizinhos (busca de 27 células) domina a física agora.
- Funções de aresta em ponto flutuante: arestas compartilhadas não têm garantia bit a bit de
  cobertura única (rasterização em ponto fixo resolveria).
- Física: só esferas e caixas (sem cápsulas, malhas ou juntas entre caixas), sem remoção de corpos
  e sem *sleeping*. Caixas, restrições e caixa-contra-partícula são seriais; com centenas de caixas
  valeria um broadphase de verdade e *islands* em paralelo.
- `raycast` contra partículas é força bruta (SIMD); com centenas de raios por tick valeria uma BVH
  ou percorrer o hash.
- O hash espacial usa uma célula do tamanho do maior diâmetro: a bola de demolição (raio 0,5) deixa
  a busca de vizinhos das bolinhas mais cara no sandbox.
- A água é desenhada opaca (o rasterizador não tem transparência).
- Fila de jobs com `Mutex` único; com milhares de jobs por frame valeria deques por thread com
  roubo de trabalho.
- Janela só no Windows; o núcleo roda em qualquer SO em modo headless.
- GPU: o `Renderer` é um trait. Um backend Vulkan/D3D12 entra no mesmo lugar do
  `SoftwareRenderer`, recebendo o mesmo `Snapshot`.
