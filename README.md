# open-td

A tower defense engine in the spirit of Warcraft III custom TD maps. **Rust is
the framework; the game is WebAssembly.** The engine knows grids, lanes,
units, structures, combat, players, teams and named resources. Waves, lives,
income, builders, versus sending and winning are wasm rule modules, and the
base game ships its own modes that way. See [ARCHITECTURE.md](ARCHITECTURE.md).

## Play

```sh
cargo xtask modules                 # build the base rule modules (wasm)
cargo run --release -p td_client    # the game (window)
```

Pick a map or the random canyon generator, set the number of players (you
are player 1, bots play the rest) and press Play.

| Input | Action |
|---|---|
| `1`–`0` or the bottom bar | pick a building; click to place, Shift-click to place more |
| left click | select a tower, builder or creep |
| right click | move the selected builder; otherwise cancel |
| `U` / `Tab` / `Del` | upgrade / cycle targeting / sell the selected tower |
| WASD, arrows, right- or middle-drag | pan |
| mouse wheel | zoom |
| Space, `+`/`-` | pause, game speed |

By default a builder walks to the site before a tower goes up (the
`builders` module). The faint lines show the route creeps take right now; a
placement that would cut every route is refused, so on Open Field you can
maze.

## Headless

```sh
cargo run --release -p td_headless -- --list
cargo run --release -p td_headless -- --map first_steps --bots
cargo run --release -p td_headless -- --generate random_canyon --param seed=7 --param lanes=2 --bots
cargo run --release -p td_headless -- --map duel --bots --record duel.tdr
cargo run --release -p td_headless -- --replay duel.tdr
```

`--bots` lets a greedy bot play every seat; it is handy for balance checks.
Replays store the map, seed and player commands, and verify the final state
hash on playback.

## Build

| Task | Command |
|---|---|
| tests | `cargo xtask modules && cargo test --workspace` |
| base pack zip | `cargo xtask pack` → `dist/base-<version>.zip` |
| web build | `cargo xtask web` → `dist/web/` (needs `wasm-bindgen-cli` matching `Cargo.lock`) |

Native builds need the usual Bevy system libraries on Linux
(`libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev`). CI builds Linux,
macOS (Apple Silicon), Windows and the web, deploys the web build to GitHub
Pages from `main`, and attaches everything to tagged releases.

## Making content

Everything in [packs/base](packs/base) is data or wasm:

- `data/*.toml`: `[[tile]]`, `[[unit]]`, `[[structure]]`, `[damage]`, `[[vfx]]`
- `maps/*.toml`: a grid and a wave table is a complete map; modules, params
  and legend default to `info.toml`'s `[defaults]`
- `modules/*.wasm`: built from [modules/](modules); write your own with
  [`td_mod_sdk`](crates/td_mod_sdk) (`export_module!`) or any language that
  speaks the ABI in [`td_mod_api`](crates/td_mod_api/src/lib.rs)

A pack of your own is a folder or zip with an `info.toml`; load it with
`--pack my_pack.zip`. It may add ids but never redefine a base id.
