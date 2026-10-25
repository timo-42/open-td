//! Turning a map into a running game: defaults, lanes, players, modules.

use std::collections::BTreeMap;

use td_core::{Game, LaneSpec, Rule, Sim, SimSetup};
use td_mod_api::{LaneOwner, LaneOwnerDef, Map, ModuleInit, PlayerInfo, Value};
use td_mods::WasmModule;

use crate::map::grid_rows;
use crate::pack::Library;

#[derive(Debug, Clone)]
pub struct GameOptions {
    /// Overrides the map's player count.
    pub players: Option<u8>,
    pub seed: u64,
}

impl Default for GameOptions {
    fn default() -> Self {
        GameOptions {
            players: None,
            seed: 1,
        }
    }
}

/// A map with the pack defaults applied.
#[derive(Debug, Clone)]
pub struct ResolvedMap {
    pub name: String,
    pub modules: Vec<String>,
    pub params: Value,
    pub players: Vec<PlayerInfo>,
    pub lanes: Vec<LaneSpec>,
    pub extra: Value,
}

pub fn resolve(lib: &Library, map: &Map, options: &GameOptions) -> Result<ResolvedMap, String> {
    let mut params = lib.defaults.params.clone();
    params.merge(&map.params);
    let mut legend = lib.defaults.legend.clone();
    legend.extend(map.legend.clone());
    let modules = map
        .modules
        .clone()
        .unwrap_or_else(|| lib.defaults.modules.clone());

    let count = options.players.unwrap_or(map.players).max(1);
    let teams = map.teams.unwrap_or(1).clamp(1, count);
    let players: Vec<PlayerInfo> = (0..count)
        .map(|i| PlayerInfo {
            id: i,
            team: (i as u32 * teams as u32 / count as u32) as u8,
            name: format!("Player {}", i + 1),
        })
        .collect();

    let mut lanes = Vec::new();
    for (li, def) in map.lanes.iter().enumerate() {
        let template =
            parse_grid(lib, &def.grid, &legend).map_err(|e| format!("lane {}: {e}", li + 1))?;
        let owners: Vec<LaneOwner> = match def.owner {
            LaneOwnerDef::Player => players.iter().map(|p| LaneOwner::Player(p.id)).collect(),
            LaneOwnerDef::Team => (0..teams).map(LaneOwner::Team).collect(),
            LaneOwnerDef::Shared => vec![LaneOwner::Shared],
        };
        for owner in owners {
            lanes.push(LaneSpec {
                owner,
                ..template.clone()
            });
        }
    }
    Ok(ResolvedMap {
        name: map.name.clone(),
        modules,
        params,
        players,
        lanes,
        extra: Value::Map(map.extra.clone()),
    })
}

fn parse_grid(
    lib: &Library,
    grid: &str,
    legend: &BTreeMap<String, String>,
) -> Result<LaneSpec, String> {
    let rows = grid_rows(grid);
    if rows.is_empty() {
        return Err("empty grid".into());
    }
    let (w, h) = (rows[0].len() as u32, rows.len() as u32);
    let mut tiles = Vec::with_capacity((w * h) as usize);
    let (mut spawns, mut exits) = (Vec::new(), Vec::new());
    for (y, row) in rows.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            let key = c.to_string();
            let id = match (legend.get(&key), c) {
                (Some(id), _) => id.as_str(),
                (None, 'S' | 'E') => "path",
                (None, _) => return Err(format!("character `{c}` is not in the legend")),
            };
            let tile = lib
                .content
                .tile(id)
                .ok_or_else(|| format!("unknown tile kind `{id}`"))?;
            tiles.push(tile);
            match c {
                'S' => spawns.push((x as i32, y as i32)),
                'E' => exits.push((x as i32, y as i32)),
                _ => {}
            }
        }
    }
    if spawns.is_empty() {
        return Err("no spawn (S)".into());
    }
    if exits.is_empty() {
        return Err("no exit (E)".into());
    }
    Ok(LaneSpec {
        owner: LaneOwner::Shared,
        width: w,
        height: h,
        tiles,
        spawns,
        exits,
    })
}

/// Seed for module `index`, independent of the simulation's own stream.
fn module_seed(seed: u64, index: usize) -> u64 {
    let mut rng = td_core::rng::Rng::new(seed ^ 0x6d6f_6475_6c65_7321);
    for _ in 0..=index {
        rng.next_u64();
    }
    rng.next_u64()
}

/// Builds the simulation and loads the map's modules.
pub fn start(lib: &Library, map: &Map, options: &GameOptions) -> Result<Game, String> {
    let resolved = resolve(lib, map, options)?;
    start_resolved(lib, &resolved, options.seed)
}

pub fn start_resolved(lib: &Library, map: &ResolvedMap, seed: u64) -> Result<Game, String> {
    let sim = Sim::new(
        lib.content.clone(),
        SimSetup {
            players: map.players.clone(),
            lanes: map.lanes.clone(),
            seed,
            default_sell_value: map.params.get("sell_value").f64_or(0.75),
        },
    )?;
    for lane in &sim.lanes {
        if !lane.fields.iter().any(|f| f.guarded) {
            return Err(format!(
                "lane {}: no unit can get from the spawn to an exit",
                lane.id + 1
            ));
        }
    }
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    for (i, name) in map.modules.iter().enumerate() {
        let bytes = lib
            .module_bytes(name)
            .ok_or_else(|| format!("unknown module `{name}`"))?;
        let mut module = WasmModule::load(name, bytes, module_seed(seed, i))?;
        if !module.has_rules() {
            return Err(format!("`{name}` is not a rule module"));
        }
        module.init(&ModuleInit {
            name: name.clone(),
            params: map.params.get(name).clone(),
            all_params: map.params.clone(),
            map: map.extra.clone(),
            data: lib.data.clone(),
            players: map.players.clone(),
        })?;
        rules.push(Box::new(module));
    }
    Ok(Game::new(sim, rules))
}
