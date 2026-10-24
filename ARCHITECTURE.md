# open-td architecture

A tower defense engine in the spirit of Warcraft III custom TD maps. **Rust is
the framework; the game is WebAssembly.** The engine knows about grids, lanes,
units, structures, movement, combat, players, teams and named resources. It
does not know what "lives", "waves", "gold", "wood", "builders" or "winning"
mean. Those come from wasm rule modules, and the base game ships its own modes
as wasm too (written in Rust, compiled to `wasm32-unknown-unknown`).

## Targets

- macOS on Apple Silicon, Linux amd64, Windows 10+ amd64
- Browser (`wasm32-unknown-unknown`), hosted on GitHub Pages

## Decisions

| Concern | Choice | Reason |
|---|---|---|
| Engine | Bevy 0.20 (pinned) for the client; plain Rust for the sim | Runs on all 4 targets |
| Simulation | `td_core`, fixed tick (30 Hz), deterministic, players and teams from day one | Headless tests, replays, lockstep multiplayer later |
| Rules | wasm modules run by **wasmi** on every target, fuel-metered | Pure Rust host, runs in the browser too, sandboxed, runaway code stops deterministically |
| Stat tables | TOML inside the pack | Units and structures are data; no code needed to add a tower |
| Mod ABI | Hand-written core-wasm ABI, MessagePack payloads | Language-neutral, works in wasmi |
| Hosting | GitHub Pages for the web build; GitHub repos as a pack registry | Static hosting is enough |

Rejected: Lua (mlua has no `wasm32-unknown-unknown`), browser-native wasm for
mods (no instruction limit; possible later as a second backend).

## Layers

```
┌───────────────────────────────────────────────────────────────┐
│ Pack (zip): info.toml, maps/, data/, modules/*.wasm, assets/  │
│   rule modules: classic.wasm, team_lives.wasm, versus.wasm,  │
│                 builders.wasm, gold_and_wood.wasm, …          │
│   map generators: canyon_gen.wasm                             │
├───────────────────────────────────────────────────────────────┤
│ td_mods  (wasmi host, fuel, Event → Command bridge)           │
├───────────────────────────────────────────────────────────────┤
│ td_core  (engine kernel, deterministic)                       │
│   grid + lanes, pathing, units, structures, movement, combat, │
│   players, teams, named resources, timers, events, commands   │
└───────────────────────────────────────────────────────────────┘
```

### What the engine (Rust) owns

- **Lanes**: a map has one or more lanes, each a tile grid with spawn and exit
  tiles. A lane has an owner (a player, a team, or shared).
- **Players and teams**: every command carries the issuing player. The engine
  only checks ownership; rules decide the rest.
- **Resources**: named integer counters per player and per team
  (`gold`, `wood`, `lives`, anything). The engine stores and reports them,
  nothing more.
- **Units**: anything that moves. Creeps, builders, sent monsters. A unit has
  an owner, stats from a data table, a lane and a position. The engine moves
  it along the lane path or to a requested cell.
- **Structures**: anything placed on a buildable tile. Towers, walls, shops.
  The engine handles placement, targeting, attack cooldown and damage.
- **Timers**: a module asks for a timer; the engine fires it as an event.
- **Hot paths**: movement, targeting, collision, damage. Never in wasm.
- **Determinism and replay**: seeded RNG, fixed tick, command log.

### What rules (wasm) own

Everything that is a *decision*: wave composition and pacing, what a leaked
creep costs and whom, income, bounty, builder count and when a second builder
unlocks, which towers a player may build, versus-mode sending, end
conditions. A rule module reacts to events and issues commands, and may keep
its own state in wasm memory.

## Hooks and commands

Events the engine emits (each is a hook a module can implement):

| Event | When |
|---|---|
| `GameStart { players, teams, lanes }` | first tick |
| `Tick { n }` | every tick, **opt-in** (costs fuel each tick) |
| `Timer { id }` | a timer requested by this module fired |
| `UnitSpawned { unit, kind, owner, lane }` | |
| `UnitDied { unit, kind, owner, lane, killer }` | killer is a structure or unit |
| `UnitReachedExit { unit, kind, owner, lane }` | engine removes the unit; rules decide the consequence |
| `UnitArrived { unit, at }` | a unit ordered to a cell got there (builders) |
| `StructureBuilt { structure, kind, owner, at }` | |
| `StructureSold { structure, owner }` | |
| `ResourceChanged { scope, name, value, delta }` | scope is a player or a team |
| `PlayerRequest { player, request }` | UI actions the engine doesn't define: `buy_builder`, `send_monster`, `upgrade` … forwarded verbatim |
| `PlayerLeft { player }` | |

Commands a module (or the player's input layer) can issue:

| Command | Effect |
|---|---|
| `Spawn { kind, lane, owner, hp_percent }` | create a unit at a lane spawn |
| `OrderMove { unit, to }` | walk a unit to a cell |
| `Build { kind, at, owner, builder? }` | place a structure, optionally requiring a builder to walk there first |
| `Sell { structure }` | |
| `SetResource { scope, name, delta }` | |
| `SetTimer { id, ticks, repeat }`, `CancelTimer { id }` | |
| `SetAllowed { player, structures: [...] }` | restrict the build palette |
| `Message { to, text }` | HUD text |
| `EndGame { result }` | result lists winners and losers; first module to end the game wins |

Order inside one tick: apply queued commands → move units → structures attack
→ resolve deaths and exits → emit events → call modules → queue their commands
for the next tick. Deterministic given the same command log.

## Game modes are just rule modules

The base pack ships these as wasm, and a map picks which to load:

| Module | What it does |
|---|---|
| `waves` | reads the map's wave table, spawns on timers, emits wave numbers as messages |
| `solo_lives` | each player has `lives`; leak costs lives; last player standing or survive all waves |
| `team_lives` | all lanes feed one shared team `lives` pool |
| `versus` | players earn `gold` income; `send_monster` request spawns a bought creep into the opposing team's lane; team with lives left wins |
| `builders` | gives each player one builder unit; `buy_builder` request adds another; `Build` must name a builder |
| `wood` | second resource with its own income rule |

A pack author combines them (`modules = ["waves", "team_lives", "builders"]`)
or replaces any with their own. There is no privileged "mode" concept in the
engine; modules run in the listed order and the first `EndGame` wins.

## Packs

```
my_pack.zip
  info.toml        id, name, version, api_version, author, generator params
  data/*.toml      [[unit]], [[structure]] stat tables
  maps/*.toml      one or more maps
  modules/*.wasm   rule modules and map generators
  assets/          sprites, audio (later)
```

A map says which modules it runs and may give them parameters:

```toml
[map]
name = "Crossroads"
modules = ["waves", "team_lives", "builders"]
players = 4                      # lanes are assigned in order

[params]                         # read by the modules, opaque to the engine
lives = 30
start.gold = 100
start.wood = 0
builders.max = 2

[[lane]]
owner = "player"                 # one lane per player
grid = """
S..######
##.######
##.......E
"""

[[wave]]
unit = "grunt"
count = 10
interval = 0.5
```

Base data (`data/*.toml`) is shared: a pack can add units and structures but
does not redefine the base ones. Everything else is decided per map through
module choice and `[params]`.

### Generated maps

A module may export a map generator. `info.toml` declares its parameters, the
client renders a form, the generator returns a map in the same shape as a map
file, and the engine validates it like any other map.

```toml
[[generator]]
id = "random_canyon"
module = "modules/canyon_gen.wasm"
params = [
  { name = "width", type = "int", min = 10, max = 40, default = 20 },
  { name = "lanes", type = "int", min = 1, max = 4, default = 1 },
  { name = "seed",  type = "int", default = 0 },
]
```

## ABI v1 (`td_mod_api`)

Guest exports: `memory`, `td_abi_version() -> u32`, `td_alloc(len) -> ptr`,
`td_free(ptr, len)`, plus any of:

- `td_init(ptr, len) -> i64`: map `[params]` in, nothing out (module state setup)
- `td_on_event(ptr, len) -> i64`: `Event` in, `Vec<Command>` out
- `td_generate_map(ptr, len) -> i64`: generator params in, `Map` out

Returns pack `ptr << 32 | len`. Host imports (module `td`): `log(ptr, len)`,
`random() -> u64` (seeded, deterministic). Each call has a fuel budget; a
module that traps or runs dry is disabled and the player is told.

## Workspace

```
crates/
  td_mod_api/   Event, Command, Map, Params types + ABI docs (no_std)
  td_mod_sdk/   guest helpers: export_module!(init, on_event, generate_map)
  td_core/      engine kernel
  td_content/   info.toml, data and map TOML, zip reading, validation
  td_mods/      wasmi host
  td_headless/  run a pack without a window (tests, balance, bots)
  td_client/    Bevy app: rendering, input, HUD, generator forms
modules/        base rule modules, each a cdylib for wasm32-unknown-unknown
  waves/ solo_lives/ team_lives/ versus/ builders/ wood/ canyon_gen/
packs/
  base/         base data, starter maps, built modules
```

## Web build notes

- No file system: base pack embedded, others fetched or dropped in (IndexedDB).
- Single-threaded; module calls and pathing must fit the frame budget.
- Audio after a user gesture. No COOP/COEP on Pages, so no wasm threads.

## Build order

1. `td_mod_api`, `td_core`, `td_headless`: lanes, units, structures, resources,
   timers, events, commands, a Rust-side test rule to prove the loop
2. `td_mods` (wasmi host) + `td_mod_sdk`; port the test rule to `modules/waves`
   and `modules/solo_lives`; now the game is playable headless from wasm
3. `td_content`: TOML + zip packs, the base pack
4. Minimal `td_client` (shapes, click to build)
5. `builders`, `team_lives`, `versus`, `wood`, `canyon_gen`
6. CI: native matrix + web build to Pages
7. Then: sprites/audio, pack registry, lockstep multiplayer
