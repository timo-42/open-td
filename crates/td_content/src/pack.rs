//! `info.toml`, `data/*.toml` and the merged library of packs.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use td_core::Content;
use td_core::content::{DamageTable, StructureKind, TileKind, UnitKind};
use td_mod_api::{Map, Value};

use crate::files::{PackFiles, stem};
use crate::map::map_from_toml;
use crate::value::{from_toml, table_to_value};

pub const BASE_PACK: &str = "base";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// What a map gets when it omits modules, params or legend entries. Only
/// the base pack sets these.
#[derive(Debug, Clone, Default)]
pub struct Defaults {
    pub modules: Vec<String>,
    pub params: Value,
    pub legend: BTreeMap<String, String>,
    pub players: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorParam {
    pub name: String,
    #[serde(rename = "type", default = "int_type")]
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// For `type = "choice"`.
    #[serde(default)]
    pub options: Vec<String>,
}

fn int_type() -> String {
    "int".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorDef {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub module: String,
    #[serde(default)]
    pub params: Vec<GeneratorParam>,
}

impl GeneratorDef {
    /// Fills defaults and clamps numbers to `min`/`max`.
    pub fn resolve_params(&self, given: &Value) -> Value {
        let mut out = Value::map();
        for p in &self.params {
            let default = p
                .default
                .as_ref()
                .map(from_toml)
                .unwrap_or(match p.kind.as_str() {
                    "bool" => Value::Bool(false),
                    "string" | "choice" => {
                        Value::Str(p.options.first().cloned().unwrap_or_default())
                    }
                    "float" => Value::Float(p.min.unwrap_or(0.0)),
                    _ => Value::Int(p.min.unwrap_or(0.0) as i64),
                });
            let mut v = match given.get(&p.name) {
                Value::Nil => default,
                v => v.clone(),
            };
            let clamp = |x: f64| {
                x.max(p.min.unwrap_or(f64::MIN))
                    .min(p.max.unwrap_or(f64::MAX))
            };
            v = match (p.kind.as_str(), &v) {
                ("int", _) => Value::Int(clamp(v.f64_or(0.0)) as i64),
                ("float", _) => Value::Float(clamp(v.f64_or(0.0))),
                _ => v,
            };
            out.insert(p.name.clone(), v);
        }
        out
    }
}

/// A client-side particle preset (`data/vfx.toml`). The simulation never
/// reads these.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxPreset {
    pub id: String,
    /// `burst` (particles from a point), `beam` (a line from attacker to
    /// target, lightning), `trail` (follows a projectile).
    #[serde(default = "burst")]
    pub style: String,
    #[serde(default)]
    pub sprite: Option<String>,
    /// Frames in a horizontal sprite sheet.
    #[serde(default = "one")]
    pub frames: u32,
    #[serde(default = "white")]
    pub color: String,
    #[serde(default)]
    pub color_end: Option<String>,
    #[serde(default = "eight")]
    pub count: u32,
    /// Seconds.
    #[serde(default = "lifetime")]
    pub lifetime: f64,
    /// Tiles per second.
    #[serde(default = "speed")]
    pub speed: f64,
    /// Degrees of the emission cone, centred on the direction of travel.
    #[serde(default = "full_circle")]
    pub spread: f64,
    /// Tiles.
    #[serde(default = "size")]
    pub size: f64,
    #[serde(default)]
    pub size_end: Option<f64>,
    /// Tiles per second², positive is "down" the screen.
    #[serde(default)]
    pub gravity: f64,
    #[serde(default)]
    pub drag: f64,
}

fn burst() -> String {
    "burst".into()
}
fn one() -> u32 {
    1
}
fn white() -> String {
    "#ffffff".into()
}
fn eight() -> u32 {
    8
}
fn lifetime() -> f64 {
    0.4
}
fn speed() -> f64 {
    2.0
}
fn full_circle() -> f64 {
    360.0
}
fn size() -> f64 {
    0.12
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataFile {
    #[serde(default)]
    tile: Vec<TileKind>,
    #[serde(default)]
    unit: Vec<UnitKind>,
    #[serde(default)]
    structure: Vec<StructureKind>,
    #[serde(default)]
    damage: Option<DamageTable>,
    #[serde(default)]
    vfx: Vec<VfxPreset>,
}

#[derive(Debug, Clone)]
pub struct MapEntry {
    pub pack: String,
    pub id: String,
    pub map: Map,
}

#[derive(Debug, Clone)]
pub struct GeneratorEntry {
    pub pack: String,
    pub def: GeneratorDef,
}

/// The base pack plus any other packs, merged and validated.
#[derive(Debug, Clone)]
pub struct Library {
    pub packs: Vec<PackMeta>,
    pub files: Vec<PackFiles>,
    pub content: Arc<Content>,
    /// Raw stat tables for modules: `{ tile = [...], unit = [...], structure = [...] }`.
    pub data: Value,
    pub vfx: BTreeMap<String, VfxPreset>,
    pub defaults: Defaults,
    pub maps: Vec<MapEntry>,
    pub generators: Vec<GeneratorEntry>,
    modules: BTreeMap<String, (usize, String)>,
}

fn parse_toml(files: &PackFiles, path: &str) -> Result<toml::Table, String> {
    let text = files.text(path)?;
    text.parse::<toml::Table>()
        .map_err(|e| format!("{path}: {e}"))
}

impl Library {
    /// Loads packs in order; the first must be the base pack. A later pack
    /// may add ids but never redefine one: a duplicate is an error, so a base
    /// update can never silently change a map.
    pub fn load(packs: Vec<PackFiles>) -> Result<Library, String> {
        let mut metas = Vec::new();
        let mut tiles = Vec::new();
        let mut units = Vec::new();
        let mut structures = Vec::new();
        let mut damage = DamageTable::default();
        let mut vfx = BTreeMap::new();
        let mut raw = Value::map();
        let mut defaults = Defaults {
            players: 1,
            ..Default::default()
        };
        let mut maps = Vec::new();
        let mut generators: Vec<GeneratorEntry> = Vec::new();
        let mut modules = BTreeMap::new();

        for (pi, files) in packs.iter().enumerate() {
            let info = parse_toml(files, "info.toml")?;
            let meta: PackMeta = toml::Value::Table(info.clone())
                .try_into()
                .map_err(|e: toml::de::Error| format!("info.toml: {e}"))?;
            if meta.api_version != td_mod_api::ABI_VERSION {
                return Err(format!(
                    "pack `{}` targets API {}, engine is {}",
                    meta.id,
                    meta.api_version,
                    td_mod_api::ABI_VERSION
                ));
            }
            if pi == 0 && meta.id != BASE_PACK {
                return Err(format!(
                    "the first pack must be `{BASE_PACK}`, got `{}`",
                    meta.id
                ));
            }
            if metas.iter().any(|m: &PackMeta| m.id == meta.id) {
                return Err(format!("pack `{}` loaded twice", meta.id));
            }
            let pack = meta.id.clone();

            if let Some(d) = info.get("defaults").and_then(|d| d.as_table()) {
                if pi != 0 {
                    return Err(format!("{pack}: only the base pack sets [defaults]"));
                }
                if let Some(m) = d.get("modules").and_then(|m| m.as_array()) {
                    defaults.modules = m
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
                if let Some(p) = d.get("params").and_then(|p| p.as_table()) {
                    defaults.params = table_to_value(p);
                }
                if let Some(l) = d.get("legend").and_then(|l| l.as_table()) {
                    for (k, v) in l {
                        defaults
                            .legend
                            .insert(k.clone(), v.as_str().unwrap_or_default().to_string());
                    }
                }
                if let Some(p) = d.get("players").and_then(|p| p.as_integer()) {
                    defaults.players = p as u8;
                }
            }
            if let Some(g) = info.get("generator") {
                let defs: Vec<GeneratorDef> = g
                    .clone()
                    .try_into()
                    .map_err(|e| format!("{pack}: [[generator]]: {e}"))?;
                for def in defs {
                    if generators.iter().any(|e| e.def.id == def.id) {
                        return Err(format!("{pack}: duplicate generator id `{}`", def.id));
                    }
                    generators.push(GeneratorEntry {
                        pack: pack.clone(),
                        def,
                    });
                }
            }

            for (path, _) in files.list("data", ".toml") {
                let table = parse_toml(files, path)?;
                let data: DataFile = toml::Value::Table(table.clone())
                    .try_into()
                    .map_err(|e| format!("{path}: {e}"))?;
                for key in ["tile", "unit", "structure"] {
                    if let Some(items) = table.get(key).and_then(|v| v.as_array()) {
                        let mut list = raw.get(key).as_array().to_vec();
                        list.extend(items.iter().map(from_toml));
                        raw.insert(key, Value::Array(list));
                    }
                }
                tiles.extend(data.tile);
                units.extend(data.unit);
                structures.extend(data.structure);
                if let Some(d) = data.damage {
                    for (row, cols) in d.0 {
                        let target = damage.0.entry(row.clone()).or_default();
                        for (col, m) in cols {
                            if target.insert(col.clone(), m).is_some() {
                                return Err(format!(
                                    "{path}: damage[{row}][{col}] is already defined"
                                ));
                            }
                        }
                    }
                }
                for v in data.vfx {
                    if vfx.contains_key(&v.id) {
                        return Err(format!("{path}: duplicate vfx id `{}`", v.id));
                    }
                    vfx.insert(v.id.clone(), v);
                }
            }

            for (path, _) in files.list("maps", ".toml") {
                let map = map_from_toml(files.text(path)?).map_err(|e| format!("{path}: {e}"))?;
                maps.push(MapEntry {
                    pack: pack.clone(),
                    id: stem(path).to_string(),
                    map,
                });
            }

            for (path, _) in files.list("modules", ".wasm") {
                let name = stem(path).to_string();
                if let Some((other, _)) = modules.get(&name) {
                    return Err(format!(
                        "{pack}: module `{name}` is already defined by `{}`",
                        metas
                            .get(*other)
                            .map(|m: &PackMeta| m.id.as_str())
                            .unwrap_or("this pack")
                    ));
                }
                modules.insert(name, (pi, path.to_string()));
            }
            metas.push(meta);
        }

        // Solo maps first, then by size of the game and name.
        maps.sort_by(|a: &MapEntry, b: &MapEntry| {
            (a.map.players, &a.map.name).cmp(&(b.map.players, &b.map.name))
        });
        let content = Content::new(tiles, units, structures, damage)?;
        validate_refs(&content, &vfx)?;
        Ok(Library {
            packs: metas,
            files: packs,
            content: Arc::new(content),
            data: raw,
            vfx,
            defaults,
            maps,
            generators,
            modules,
        })
    }

    /// Loads `base` from a directory or zip and any extra packs.
    #[cfg(feature = "fs")]
    pub fn open(base: &std::path::Path, extra: &[std::path::PathBuf]) -> Result<Library, String> {
        let mut packs = vec![PackFiles::open(base)?];
        for p in extra {
            packs.push(PackFiles::open(p)?);
        }
        Library::load(packs)
    }

    pub fn module_bytes(&self, name: &str) -> Option<&[u8]> {
        let (pack, path) = self.modules.get(name)?;
        self.files[*pack].get(path)
    }

    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    /// `"crossroads"` or `"mypack:crossroads"`.
    pub fn find_map(&self, id: &str) -> Option<&MapEntry> {
        match id.split_once(':') {
            Some((pack, id)) => self.maps.iter().find(|m| m.pack == pack && m.id == id),
            None => self.maps.iter().find(|m| m.id == id),
        }
    }

    pub fn find_generator(&self, id: &str) -> Option<&GeneratorEntry> {
        self.generators.iter().find(|g| g.def.id == id)
    }

    /// Runs a generator module. The map it returns is validated like any
    /// other when a game starts.
    pub fn generate(&self, id: &str, params: &Value) -> Result<Map, String> {
        let entry = self
            .find_generator(id)
            .ok_or_else(|| format!("unknown generator `{id}`"))?;
        let pi = self.packs.iter().position(|p| p.id == entry.pack).unwrap();
        let path = entry.def.module.trim_start_matches("./");
        let bytes = self.files[pi]
            .get(path)
            .or_else(|| self.module_bytes(stem(path)))
            .ok_or_else(|| format!("generator `{id}`: missing {path}"))?;
        let params = entry.def.resolve_params(params);
        let seed = params.get("seed").i64_or(0) as u64;
        let mut module = td_mods::WasmModule::load(stem(path), bytes, seed)?;
        module.generate_map(&params)
    }
}

fn validate_refs(content: &Content, vfx: &BTreeMap<String, VfxPreset>) -> Result<(), String> {
    let check = |owner: &str, id: &Option<String>| -> Result<(), String> {
        match id {
            Some(id) if !vfx.contains_key(id) => {
                Err(format!("`{owner}` references unknown vfx `{id}`"))
            }
            _ => Ok(()),
        }
    };
    for s in &content.structures {
        if let Some(v) = &s.vfx {
            check(&s.id, &v.attack)?;
            check(&s.id, &v.projectile)?;
            check(&s.id, &v.hit)?;
        }
        for e in &s.on_hit {
            check(&s.id, &e.vfx)?;
        }
        if let Some(a) = &s.aura {
            for e in &a.effects {
                check(&s.id, &e.vfx)?;
            }
        }
    }
    for u in &content.units {
        if let Some(v) = &u.vfx {
            check(&u.id, &v.attack)?;
            check(&u.id, &v.projectile)?;
            check(&u.id, &v.hit)?;
        }
    }
    Ok(())
}
