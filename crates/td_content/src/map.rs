//! Map files: `[map]`, `[params]`, `[legend]`, `[[lane]]`, plus any other
//! tables (`[[wave]]` …) passed to the modules untouched.

use td_mod_api::{LaneDef, LaneOwnerDef, Map};

use crate::value::{from_toml, to_toml};

pub fn map_from_toml(text: &str) -> Result<Map, String> {
    let mut table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let header = table
        .remove("map")
        .and_then(|m| m.as_table().cloned())
        .unwrap_or_default();
    let name = header
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled")
        .to_string();
    let modules = match header.get("modules") {
        None => None,
        Some(v) => Some(
            v.as_array()
                .ok_or("[map] modules must be a list")?
                .iter()
                .map(|m| {
                    m.as_str()
                        .map(String::from)
                        .ok_or("[map] modules must be strings")
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let players = header
        .get("players")
        .and_then(|v| v.as_integer())
        .unwrap_or(1);
    if !(1..=16).contains(&players) {
        return Err("[map] players must be 1 to 16".into());
    }
    let teams = header
        .get("teams")
        .and_then(|v| v.as_integer())
        .map(|t| t as u8);
    let params = table
        .remove("params")
        .map(|p| from_toml(&p))
        .unwrap_or_default();
    let mut legend = std::collections::BTreeMap::new();
    if let Some(l) = table.remove("legend") {
        for (k, v) in l.as_table().ok_or("[legend] must be a table")? {
            if k.chars().count() != 1 {
                return Err(format!("legend key `{k}` must be one character"));
            }
            legend.insert(
                k.clone(),
                v.as_str().ok_or("legend values are tile ids")?.to_string(),
            );
        }
    }
    let mut lanes = Vec::new();
    for lane in table
        .remove("lane")
        .and_then(|l| l.as_array().cloned())
        .unwrap_or_default()
    {
        let owner = match lane
            .get("owner")
            .and_then(|o| o.as_str())
            .unwrap_or("player")
        {
            "player" => LaneOwnerDef::Player,
            "team" => LaneOwnerDef::Team,
            "shared" => LaneOwnerDef::Shared,
            o => return Err(format!("lane owner `{o}` (expected player, team, shared)")),
        };
        let grid = lane
            .get("grid")
            .and_then(|g| g.as_str())
            .ok_or("[[lane]] needs a grid")?
            .to_string();
        lanes.push(LaneDef { owner, grid });
    }
    if lanes.is_empty() {
        return Err("a map needs at least one [[lane]]".into());
    }
    let extra = table
        .iter()
        .map(|(k, v)| (k.clone(), from_toml(v)))
        .collect();
    Ok(Map {
        name,
        modules,
        players: players as u8,
        teams,
        params,
        legend,
        lanes,
        extra,
    })
}

/// Writes a map back as TOML, e.g. to save a generated map.
pub fn map_to_toml(map: &Map) -> String {
    let mut root = toml::Table::new();
    let mut header = toml::Table::new();
    header.insert("name".into(), map.name.clone().into());
    if let Some(m) = &map.modules {
        header.insert(
            "modules".into(),
            toml::Value::Array(m.iter().map(|s| s.clone().into()).collect()),
        );
    }
    header.insert("players".into(), (map.players as i64).into());
    if let Some(t) = map.teams {
        header.insert("teams".into(), (t as i64).into());
    }
    root.insert("map".into(), header.into());
    if let Some(p) = to_toml(&map.params).filter(|p| p.as_table().is_some_and(|t| !t.is_empty())) {
        root.insert("params".into(), p);
    }
    if !map.legend.is_empty() {
        root.insert(
            "legend".into(),
            toml::Value::Table(
                map.legend
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone().into()))
                    .collect(),
            ),
        );
    }
    let lanes = map
        .lanes
        .iter()
        .map(|l| {
            let mut t = toml::Table::new();
            let owner = match l.owner {
                LaneOwnerDef::Player => "player",
                LaneOwnerDef::Team => "team",
                LaneOwnerDef::Shared => "shared",
            };
            t.insert("owner".into(), owner.into());
            t.insert("grid".into(), l.grid.clone().into());
            toml::Value::Table(t)
        })
        .collect();
    root.insert("lane".into(), toml::Value::Array(lanes));
    for (k, v) in &map.extra {
        if let Some(v) = to_toml(v) {
            root.insert(k.clone(), v);
        }
    }
    toml::to_string(&root).unwrap_or_default()
}

/// Parses a lane grid into rows of characters. Leading and trailing blank
/// lines are dropped; short rows are padded with spaces.
pub fn grid_rows(grid: &str) -> Vec<Vec<char>> {
    let mut rows: Vec<Vec<char>> = grid
        .lines()
        .map(|l| l.trim_end_matches('\r').chars().collect())
        .collect();
    while rows
        .first()
        .is_some_and(|r| r.iter().all(|c| c.is_whitespace()))
    {
        rows.remove(0);
    }
    while rows
        .last()
        .is_some_and(|r| r.iter().all(|c| c.is_whitespace()))
    {
        rows.pop();
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for r in &mut rows {
        r.resize(width, ' ');
    }
    rows
}
