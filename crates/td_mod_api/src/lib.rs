//! Types shared by the open-td engine and wasm rule modules.
//!
//! # ABI v1
//!
//! A module is a core wasm module. It exports:
//!
//! - `memory`
//! - `td_abi_version() -> u32`, returning [`ABI_VERSION`]
//! - `td_alloc(len: u32) -> u32` and `td_free(ptr: u32, len: u32)`
//!
//! and any of:
//!
//! - `td_init(ptr, len) -> i64`: [`ModuleInit`] in, nothing out
//! - `td_on_event(ptr, len) -> i64`: [`Event`] in, `Vec<Command>` out
//! - `td_generate_map(ptr, len) -> i64`: [`Value`] generator params in, [`Map`] out
//!
//! Inputs are written by the host into a buffer obtained from `td_alloc`;
//! the guest owns and frees it. Outputs are returned as `ptr << 32 | len`
//! (`0` means "nothing") and the host frees them with `td_free` after
//! reading. A negative return is an error; the module is disabled.
//!
//! All payloads are MessagePack with struct fields as maps (named keys) and
//! enums externally tagged: `{"Spawn": {"kind": "grunt", ...}}`, unit
//! variants as plain strings.
//!
//! Host imports, module `td`: `log(ptr: u32, len: u32)` (UTF-8 text) and
//! `random() -> i64` (deterministic, seeded per game).

#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

mod value;
pub use value::Value;

#[cfg(feature = "codec")]
pub mod codec;

pub const ABI_VERSION: u32 = 1;

/// Ticks per second of the simulation.
pub const TICK_RATE: u32 = 30;

pub type PlayerId = u8;
pub type TeamId = u8;
pub type LaneId = u16;

/// The owner of wave creeps; hostile to every team.
pub const NEUTRAL: PlayerId = 255;
/// The team of [`NEUTRAL`].
pub const NEUTRAL_TEAM: TeamId = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StructureId(pub u32);

/// A tile on a lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Cell {
    pub lane: LaneId,
    pub x: i32,
    pub y: i32,
}

impl Cell {
    pub const fn new(lane: LaneId, x: i32, y: i32) -> Self {
        Cell { lane, x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    Player(PlayerId),
    Team(TeamId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LaneOwner {
    Player(PlayerId),
    Team(TeamId),
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Killer {
    Structure(StructureId),
    Unit(UnitId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Targeting {
    #[default]
    First,
    Last,
    Nearest,
    Strongest,
    Weakest,
}

impl Targeting {
    pub const ALL: [Targeting; 5] = [
        Targeting::First,
        Targeting::Last,
        Targeting::Nearest,
        Targeting::Strongest,
        Targeting::Weakest,
    ];

    pub fn parse(s: &str) -> Option<Targeting> {
        Some(match s {
            "first" => Targeting::First,
            "last" => Targeting::Last,
            "nearest" => Targeting::Nearest,
            "strongest" => Targeting::Strongest,
            "weakest" => Targeting::Weakest,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Targeting::First => "first",
            Targeting::Last => "last",
            Targeting::Nearest => "nearest",
            Targeting::Strongest => "strongest",
            Targeting::Weakest => "weakest",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub team: TeamId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneInfo {
    pub id: LaneId,
    pub owner: LaneOwner,
    pub width: u32,
    pub height: u32,
    pub spawns: Vec<Cell>,
    pub exits: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageTarget {
    All,
    Player(PlayerId),
    Team(TeamId),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GameResult {
    pub winners: Vec<PlayerId>,
    pub losers: Vec<PlayerId>,
    #[serde(default)]
    pub text: String,
}

/// Something that happened in the engine. Every variant is a hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    GameStart {
        players: Vec<PlayerInfo>,
        lanes: Vec<LaneInfo>,
    },
    /// Only delivered to modules that issued `SetTickEvents { enabled: true }`.
    Tick {
        n: u64,
    },
    /// Only delivered to the module that set the timer.
    Timer {
        id: u32,
    },
    UnitSpawned {
        unit: UnitId,
        kind: String,
        owner: PlayerId,
        lane: LaneId,
    },
    UnitDied {
        unit: UnitId,
        kind: String,
        owner: PlayerId,
        lane: LaneId,
        killer: Option<Killer>,
        /// Owner of the killer, so bounty modules need not track structures.
        killer_owner: Option<PlayerId>,
    },
    /// The engine has already removed the unit.
    UnitReachedExit {
        unit: UnitId,
        kind: String,
        owner: PlayerId,
        lane: LaneId,
    },
    UnitArrived {
        unit: UnitId,
        at: Cell,
    },
    /// Placed (and, with a `build_time`, finished constructing) or upgraded.
    StructureBuilt {
        structure: StructureId,
        kind: String,
        owner: PlayerId,
        at: Cell,
    },
    StructureSold {
        structure: StructureId,
        owner: PlayerId,
    },
    /// A structure with `hp` was destroyed by a unit.
    StructureDestroyed {
        structure: StructureId,
        kind: String,
        owner: PlayerId,
        killer: Option<UnitId>,
    },
    ResourceChanged {
        scope: Scope,
        name: String,
        value: i64,
        delta: i64,
    },
    /// A UI action the engine doesn't define, forwarded verbatim.
    PlayerRequest {
        player: PlayerId,
        request: String,
        #[serde(default)]
        data: Value,
    },
    PlayerLeft {
        player: PlayerId,
    },
    /// Broadcast by a module through `Command::Signal`; lets modules talk
    /// (`waves` signals `waves_done`, `lives` ends the game on it).
    Signal {
        name: String,
        #[serde(default)]
        data: Value,
    },
}

/// Something a module or a player asks the engine to do.
///
/// Players may issue `OrderMove`, `Build`, `Sell`, `Upgrade`,
/// `SetTargeting` and `Request`, only for things they own. Modules may issue
/// anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Create a unit at a lane spawn (or at `at`). An `idle` unit stays
    /// where it is until ordered; otherwise it walks to the exit.
    Spawn {
        kind: String,
        lane: LaneId,
        owner: PlayerId,
        #[serde(default = "hundred")]
        hp_percent: u32,
        #[serde(default)]
        at: Option<Cell>,
        #[serde(default)]
        idle: bool,
    },
    OrderMove {
        unit: UnitId,
        to: Cell,
    },
    /// Place a structure. With a `builder`, the builder walks there first.
    Build {
        kind: String,
        at: Cell,
        owner: PlayerId,
        #[serde(default)]
        builder: Option<UnitId>,
    },
    Sell {
        structure: StructureId,
    },
    /// Replace a structure in place with one of its `upgrades_to`.
    Upgrade {
        structure: StructureId,
        to: String,
    },
    SetTargeting {
        structure: StructureId,
        mode: Targeting,
    },
    /// Fails if a counter would go negative, unless `allow_negative`.
    AddResource {
        scope: Scope,
        name: String,
        delta: i64,
        #[serde(default)]
        allow_negative: bool,
    },
    /// Deduct `cost` from `scope` and run `then`, or do nothing and tell
    /// `notify` that it is not affordable.
    Purchase {
        scope: Scope,
        cost: BTreeMap<String, i64>,
        then: Vec<Command>,
        #[serde(default)]
        notify: Option<PlayerId>,
    },
    SetTimer {
        id: u32,
        ticks: u32,
        #[serde(default)]
        repeat: bool,
    },
    CancelTimer {
        id: u32,
    },
    /// Restrict the build palette; `None` allows everything.
    SetAllowed {
        player: PlayerId,
        structures: Option<Vec<String>>,
    },
    /// When set, `Build` from this player must name one of their units.
    SetRequireBuilder {
        player: PlayerId,
        required: bool,
    },
    /// Opt in or out of `Tick` events for the issuing module.
    SetTickEvents {
        enabled: bool,
    },
    Message {
        to: MessageTarget,
        text: String,
    },
    Signal {
        name: String,
        #[serde(default)]
        data: Value,
    },
    /// First one wins.
    EndGame {
        result: GameResult,
    },
    /// Player only: becomes a `PlayerRequest` event.
    Request {
        request: String,
        #[serde(default)]
        data: Value,
    },
}

fn hundred() -> u32 {
    100
}

/// What `td_init` receives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleInit {
    /// The module's own name, as listed in the map.
    pub name: String,
    /// `[params.<name>]` of the map, merged over the pack defaults.
    pub params: Value,
    /// All of `[params]`, for modules that share settings.
    pub all_params: Value,
    /// The map's extra top-level tables (`wave`, …).
    pub map: Value,
    /// Raw stat tables: `{ unit = [...], structure = [...] }`.
    pub data: Value,
    pub players: Vec<PlayerInfo>,
}

/// Who a `[[lane]]` belongs to in a map file; expanded at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneOwnerDef {
    /// One copy of the lane per player.
    #[default]
    Player,
    /// One copy per team.
    Team,
    /// A single lane everybody builds on.
    Shared,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneDef {
    #[serde(default)]
    pub owner: LaneOwnerDef,
    /// Rows of legend characters; `S` is a spawn, `E` an exit.
    pub grid: String,
}

/// A map, as written in `maps/*.toml` or returned by a generator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Map {
    pub name: String,
    #[serde(default)]
    pub modules: Option<Vec<String>>,
    #[serde(default = "one")]
    pub players: u8,
    /// Number of teams; players are split into contiguous blocks. Default 1
    /// (everybody together).
    #[serde(default)]
    pub teams: Option<u8>,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub legend: BTreeMap<String, String>,
    pub lanes: Vec<LaneDef>,
    /// Other top-level tables, such as `wave`; opaque to the engine.
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
}

fn one() -> u8 {
    1
}
