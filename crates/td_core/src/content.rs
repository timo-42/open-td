//! Stat tables. Everything here runs every tick, so it lives in Rust; the
//! decisions around it live in rule modules.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use td_mod_api::{TICK_RATE, Targeting};

use crate::flags::TileFlags;

/// Seconds to ticks, rounded, at least `min`.
pub fn secs_to_ticks(secs: f64, min: u32) -> u32 {
    let t = (secs * TICK_RATE as f64 + 0.5).floor();
    if t < min as f64 { min } else { t as u32 }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileKind {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub flags: TileFlags,
    #[serde(default)]
    pub sprite: Option<String>,
    /// Placeholder colour for clients without sprites, `"#rrggbb"`.
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl UnitSize {
    /// Radius in tiles, used for hit tests and drawing.
    pub fn radius(self) -> f64 {
        match self {
            UnitSize::Small => 0.2,
            UnitSize::Medium => 0.3,
            UnitSize::Large => 0.42,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    /// Movement speed reduced by `percent`.
    Slow,
    /// `dps` damage per second, ignoring armor.
    Dot,
    /// No movement, no attacks.
    Stun,
    /// Flat armor modifier `amount`, may be negative.
    Armor,
    /// On hit: jumps to `count` more targets within `range`, each taking
    /// `falloff` percent of the previous hit.
    Bounce,
    /// Structures only (auras): damage up by `percent`.
    Damage,
    /// Structures only (auras): attack speed up by `percent`.
    Haste,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDef {
    pub kind: EffectKind,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub vfx: Option<String>,
    #[serde(default)]
    pub percent: f64,
    #[serde(default)]
    pub dps: f64,
    #[serde(default)]
    pub amount: f64,
    #[serde(default)]
    pub count: u32,
    #[serde(default = "default_falloff")]
    pub falloff: f64,
    #[serde(default = "default_bounce_range")]
    pub range: f64,
}

fn default_falloff() -> f64 {
    70.0
}
fn default_bounce_range() -> f64 {
    2.5
}

impl EffectDef {
    /// "The strongest effect of a kind wins."
    pub fn strength(&self) -> f64 {
        match self.kind {
            EffectKind::Slow | EffectKind::Damage | EffectKind::Haste => self.percent,
            EffectKind::Dot => self.dps,
            EffectKind::Stun => self.duration,
            EffectKind::Armor => self.amount.abs(),
            EffectKind::Bounce => self.count as f64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuraFilter {
    /// Friendly structures.
    Allies,
    /// Hostile units.
    #[default]
    Enemies,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aura {
    pub radius: f64,
    #[serde(default)]
    pub filter: AuraFilter,
    pub effects: Vec<EffectDef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VfxRefs {
    #[serde(default)]
    pub attack: Option<String>,
    #[serde(default)]
    pub projectile: Option<String>,
    #[serde(default)]
    pub hit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitAttack {
    pub range: f64,
    pub damage: f64,
    pub cooldown: f64,
    /// Which structures it hits, by the flags they block. Default: any.
    #[serde(default)]
    pub targets: Option<TileFlags>,
    #[serde(default = "default_damage_type")]
    pub damage_type: String,
}

fn default_damage_type() -> String {
    "normal".into()
}
fn default_armor_type() -> String {
    "light".into()
}
fn default_structure_armor_type() -> String {
    "fortified".into()
}
fn one_u8() -> u8 {
    1
}
fn ground() -> TileFlags {
    TileFlags::GROUND
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitKind {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sprite: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    pub hp: f64,
    pub speed: f64,
    #[serde(default)]
    pub armor: f64,
    #[serde(default = "default_armor_type")]
    pub armor_type: String,
    #[serde(default = "ground")]
    pub moves_on: TileFlags,
    #[serde(default)]
    pub size: UnitSize,
    /// Paid by the `income` module, not the engine.
    #[serde(default)]
    pub bounty: BTreeMap<String, i64>,
    #[serde(default)]
    pub immune_to: Vec<EffectKind>,
    #[serde(default)]
    pub attack: Option<UnitAttack>,
    #[serde(default)]
    pub vfx: Option<VfxRefs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureKind {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sprite: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub cost: BTreeMap<String, i64>,
    #[serde(default = "one_u8")]
    pub size: u8,
    #[serde(default = "ground")]
    pub blocks: TileFlags,
    #[serde(default)]
    pub targets: TileFlags,
    #[serde(default)]
    pub range: f64,
    #[serde(default)]
    pub damage: f64,
    #[serde(default = "one_f64")]
    pub cooldown: f64,
    #[serde(default = "default_damage_type")]
    pub damage_type: String,
    #[serde(default, with = "targeting_serde")]
    pub targeting: Targeting,
    /// Projectile speed in tiles per second; absent is an instant hit.
    #[serde(default)]
    pub projectile: Option<f64>,
    #[serde(default)]
    pub vfx: Option<VfxRefs>,
    #[serde(default)]
    pub splash: Option<f64>,
    /// Damage percent at the splash edge.
    #[serde(default = "hundred")]
    pub splash_falloff: f64,
    #[serde(default)]
    pub build_time: f64,
    #[serde(default)]
    pub sell_value: Option<f64>,
    #[serde(default)]
    pub upgrades_to: Vec<String>,
    #[serde(default)]
    pub hp: Option<f64>,
    #[serde(default)]
    pub armor: f64,
    #[serde(default = "default_structure_armor_type")]
    pub armor_type: String,
    #[serde(default)]
    pub on_hit: Vec<EffectDef>,
    #[serde(default)]
    pub aura: Option<Aura>,
}

fn one_f64() -> f64 {
    1.0
}
fn hundred() -> f64 {
    100.0
}

impl StructureKind {
    pub fn can_attack(&self) -> bool {
        self.damage > 0.0 && !self.targets.is_empty() && self.range > 0.0
    }
}

mod targeting_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use td_mod_api::Targeting;

    pub fn serialize<S: Serializer>(t: &Targeting, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(t.name())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Targeting, D::Error> {
        let s = String::deserialize(d)?;
        Targeting::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown targeting `{s}`")))
    }
}

/// Damage multipliers: rows are damage types, columns armor types. Missing
/// entries are 1.0.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DamageTable(pub BTreeMap<String, BTreeMap<String, f64>>);

impl DamageTable {
    pub fn multiplier(&self, damage_type: &str, armor_type: &str) -> f64 {
        self.0
            .get(damage_type)
            .and_then(|row| row.get(armor_type))
            .copied()
            .unwrap_or(1.0)
    }
}

/// Warcraft III style armor: each point reduces damage by about 6%,
/// negative armor amplifies it. Only basic arithmetic, so it is bit-exact
/// on every target.
pub fn armor_factor(armor: f64) -> f64 {
    if armor >= 0.0 {
        1.0 - (0.06 * armor) / (1.0 + 0.06 * armor)
    } else {
        let mut f = 1.0;
        let n = (-armor).min(50.0) as u32;
        for _ in 0..n {
            f *= 0.94;
        }
        2.0 - f
    }
}

/// All kinds available to a game, base pack first. Indices are stable for
/// the life of a game.
#[derive(Debug, Clone, Default)]
pub struct Content {
    pub tiles: Vec<TileKind>,
    pub units: Vec<UnitKind>,
    pub structures: Vec<StructureKind>,
    pub damage: DamageTable,
    tile_index: BTreeMap<String, usize>,
    unit_index: BTreeMap<String, usize>,
    structure_index: BTreeMap<String, usize>,
}

impl Content {
    pub fn new(
        tiles: Vec<TileKind>,
        units: Vec<UnitKind>,
        structures: Vec<StructureKind>,
        damage: DamageTable,
    ) -> Result<Content, String> {
        fn index<T>(
            items: &[T],
            id: impl Fn(&T) -> &str,
            what: &str,
        ) -> Result<BTreeMap<String, usize>, String> {
            let mut out = BTreeMap::new();
            for (i, item) in items.iter().enumerate() {
                if out.insert(id(item).to_string(), i).is_some() {
                    return Err(format!("duplicate {what} id `{}`", id(item)));
                }
            }
            Ok(out)
        }
        let tile_index = index(&tiles, |t| &t.id, "tile")?;
        let unit_index = index(&units, |t| &t.id, "unit")?;
        let structure_index = index(&structures, |t| &t.id, "structure")?;
        for s in &structures {
            for u in &s.upgrades_to {
                if !structure_index.contains_key(u) {
                    return Err(format!("structure `{}` upgrades to unknown `{u}`", s.id));
                }
            }
            if s.size == 0 {
                return Err(format!("structure `{}` has size 0", s.id));
            }
        }
        Ok(Content {
            tiles,
            units,
            structures,
            damage,
            tile_index,
            unit_index,
            structure_index,
        })
    }

    pub fn tile(&self, id: &str) -> Option<usize> {
        self.tile_index.get(id).copied()
    }
    pub fn unit(&self, id: &str) -> Option<usize> {
        self.unit_index.get(id).copied()
    }
    pub fn structure(&self, id: &str) -> Option<usize> {
        self.structure_index.get(id).copied()
    }
}
