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
| Rendering | Top-down 2D sprites on a square grid; ranges and paths as overlays | Modders ship a PNG, not a rigged model; stays fast in the browser; TD is read as a grid. Isometric looks are an art choice, not an engine mode. 3D would be a separate client against the same sim |
| Simulation | `td_core`, fixed tick (30 Hz), deterministic, players and teams from day one | Headless tests, replays, lockstep multiplayer later |
| Web graphics | WebGL2 only; shaders and features stay within WebGL2 limits | Reaches every browser; 2D needs nothing more; Bevy picks the backend at compile time, so one wasm artifact. A WebGPU build can be added later as a second artifact with a loader fallback |
| Particles | Small in-house 2D system in `td_client`, CPU-simulated, sprite-batched, presets defined in pack data (`data/vfx.toml`) | bevy_hanabi needs compute shaders, which WebGL2 lacks; our counts are small; presets as data let modders add fire, lightning, wind, debris with sprite sheets. Never touches `td_core` |
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
│   rule modules: waves.wasm, lives.wasm, versus.wasm,          │
│                 builders.wasm, income.wasm, …                 │
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
- **Tiles** carry a set of flags, nothing else:

  | Flag | Meaning |
  |---|---|
  | `ground` | ground units may walk here |
  | `air` | flying units may move here |
  | `unit` | player-controlled units (builders, heroes) may walk here |
  | `build` | structures may be placed here |

  Every unit kind has a `moves_on` mask. Every structure kind has a `targets`
  mask (what it can shoot) and a `blocks` mask (which flags it removes from
  its tile while it stands; default `ground`, a tall wall might be
  `ground + air`). Pathfinding is one algorithm: a flow field per mask in
  use, computed from the exit over tiles matching the mask. A flyer is a
  unit whose mask is `air`; it is not a special case anywhere else. Placing
  or selling a structure recomputes the affected flow fields, and a placement
  that would leave a spawn with no route for any unit mask in play is
  refused. That is all a mazing map needs.
- **Players and teams**: every command carries the issuing player. The engine
  only checks ownership; rules decide the rest.
- **Resources**: named integer counters per player and per team. Names are
  opaque strings chosen by the map or module: `gold`, `wood`, `mana`, `lives`,
  `souls`, whatever the map wants. The engine stores them, reports changes and
  lets structures and units declare costs in any of them. It never
  special-cases a name.
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
| `AddResource { scope, name, delta }` | any name; scope is a player or a team; fails if it would go negative unless `allow_negative` |
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
| `lives` | a leak subtracts from a resource (name and scope from params: per player, or shared per team); whoever still has some left when the waves end, or last standing, wins |
| `income` | periodic income and kill bounty for any set of resources, amounts from params |
| `versus` | `send_monster` request spends a resource and spawns the bought creep into the opposing team's lane |
| `builders` | gives each player one builder unit; `buy_builder` request spends a resource and adds another; `Build` must name a builder |

Resource names are never hard-coded in the engine or in these modules; each
takes them from `[params]`. Solo lanes versus shared team lives is the same
`lives` module with a different scope. A pack author combines modules
(`modules = ["waves", "lives", "income", "builders"]`) or replaces any with
their own. There is no privileged "mode" concept in the
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
lives.resource = "lives"         # any name; "hp", "castle_health", …
lives.scope = "team"             # or "player"
lives.start = 30
income.start = { gold = 100, mana = 0 }
income.per_wave = { gold = 20, mana = 5 }
builders.max = 2
builders.cost = { gold = 50 }

[tiles]                          # legend; these four are the defaults
"." = ["ground", "air", "unit"]            # path
"#" = ["air", "unit", "build"]             # buildable ground
"=" = ["ground", "air", "unit", "build"]   # mazing ground
" " = []                                   # blocked, even for air

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

Stat tables declare costs as a table of resource name to amount, so a tower
can cost `{ gold = 40, mana = 10 }` and the engine checks and deducts
whatever is listed. See the stat tables section below.

Base data (`data/*.toml`) is shared: a pack can add units and structures but
does not redefine the base ones. Everything else is decided per map through
module choice and `[params]`.

### Stat tables (`data/*.toml`)

Units and structures are data. The engine implements every attribute below in
Rust because they run every tick. Decisions (unlocks, build limits, income,
crits, on-kill effects) are not here; modules do those through events and
commands.

Structure kind:

| Attribute | Notes |
|---|---|
| `id`, `name`, `sprite` | the engine never reads `sprite`; the client does |
| `cost` | resource name to amount |
| `size` | footprint in tiles, default `1`; `2` means 2x2 |
| `blocks`, `targets` | tile flag masks |
| `range` | tiles, measured from the footprint centre |
| `damage`, `cooldown`, `damage_type` | per shot; `damage_type` is a key in the pack's damage-vs-armor table, default `normal` |
| `targeting` | `first`, `last`, `nearest`, `strongest`, `weakest`; the player may switch it |
| `projectile` | absent means instant hit; otherwise speed in tiles per second, and a projectile whose target dies first is wasted |
| `vfx` | optional `{ attack, projectile, hit }` preset ids from `data/vfx.toml`; client-only |
| `splash`, `splash_falloff` | radius in tiles and damage percent at the edge; absent means single target |
| `build_time` | seconds, default `0`; builder modules rely on it |
| `sell_value` | fraction of cost; default comes from map params |
| `upgrades_to` | structure ids; upgrade replaces in place |
| `hp`, `armor`, `armor_type` | optional; a structure without `hp` cannot be targeted |
| `on_hit` | list of effects applied to the target |
| `aura` | an effect list with a radius and a filter (`allies`, `enemies`), applied continuously |

Unit kind:

| Attribute | Notes |
|---|---|
| `id`, `name`, `sprite` | |
| `hp`, `speed`, `armor`, `armor_type` | speed in tiles per second |
| `moves_on` | tile flag mask; `["air"]` is a flyer |
| `size` | `small`, `medium`, `large` for targeting and sprites |
| `bounty` | resource name to amount, paid by the `income` module, not the engine |
| `immune_to` | effect kinds this unit ignores |
| `attack` | optional `{ range, damage, cooldown, targets }` so sent creeps can hit structures with `hp` |

Effects are data too. Each has a `kind`, a duration in seconds, an optional
`vfx` preset shown on the affected unit, and kind parameters. The engine ships these kinds: `slow` (percent), `dot` (damage per
second), `stun`, `armor` (flat modifier, may be negative), `bounce` (count and
damage falloff). The strongest effect of a kind wins; a reapplication
refreshes its duration.

```toml
[[structure]]
id = "frost_tower"
cost = { gold = 60 }
range = 3
damage = 8
cooldown = 1.0
damage_type = "magic"
targets = ["ground", "air"]
projectile = 12
on_hit = [{ kind = "slow", percent = 40, duration = 2.0 }]
upgrades_to = ["blizzard_tower"]

[[unit]]
id = "wyvern"
hp = 300
speed = 2.5
armor = 2
armor_type = "light"
moves_on = ["air"]
size = "medium"
bounty = { gold = 12 }

[damage]                          # multiplier table, rows are damage types
normal = { light = 1.0, heavy = 1.0, fortified = 0.7 }
magic  = { light = 1.25, heavy = 0.75, fortified = 0.5 }
```

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
  waves/ lives/ income/ versus/ builders/ canyon_gen/
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
   and `modules/lives`; now the game is playable headless from wasm
3. `td_content`: TOML + zip packs, the base pack
4. Minimal `td_client` (shapes, click to build)
5. `income`, `builders`, `versus`, `canyon_gen`
6. CI: native matrix + web build to Pages
7. Then: sprites/audio, pack registry, lockstep multiplayer
