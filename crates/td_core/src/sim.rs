//! The deterministic simulation.
//!
//! Order inside one tick: apply queued commands → timers → construction →
//! move units → effects, auras, attacks, projectiles → resolve deaths and
//! exits → emit events. The caller ([`crate::Game`]) then hands the events
//! to the rule modules and queues their commands for the next tick.
//!
//! All arithmetic is `f64` with basic operations and `sqrt` only, which are
//! bit-exact under IEEE 754 on every supported target; containers that are
//! iterated are ordered (`BTreeMap`, `Vec`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use td_mod_api::{
    Cell, Command, Event, GameResult, Killer, LaneId, LaneInfo, LaneOwner, MessageTarget, NEUTRAL,
    NEUTRAL_TEAM, PlayerId, PlayerInfo, Scope, StructureId, TICK_RATE, Targeting, TeamId, UnitId,
};

use crate::content::{AuraFilter, Content, EffectDef, EffectKind, armor_factor, secs_to_ticks};
use crate::flags::TileFlags;
use crate::lane::{Lane, LaneSpec, UNREACHABLE};
use crate::rng::Rng;

/// Who issued a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    Player(PlayerId),
    /// Index into the game's module list.
    Module(usize),
    /// The host itself (tests, tools); may do anything.
    System,
}

/// Who an event is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    All,
    Module(usize),
    TickSubscribers,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    pub audience: Audience,
    pub event: Event,
}

/// Visual hints for clients. Never read by the simulation.
#[derive(Debug, Clone, PartialEq)]
pub enum Fx {
    /// A structure fired. `kind` is the structure kind index.
    Attack {
        structure: StructureId,
        kind: usize,
        lane: LaneId,
        from: (f64, f64),
        to: (f64, f64),
    },
    Hit {
        kind: usize,
        lane: LaneId,
        at: (f64, f64),
        splash: f64,
    },
    Bounce {
        kind: usize,
        lane: LaneId,
        from: (f64, f64),
        to: (f64, f64),
    },
    UnitAttack {
        unit: UnitId,
        lane: LaneId,
        from: (f64, f64),
        to: (f64, f64),
    },
    UnitDied {
        kind: usize,
        lane: LaneId,
        at: (f64, f64),
    },
    UnitLeaked {
        kind: usize,
        lane: LaneId,
        at: (f64, f64),
    },
    StructurePlaced {
        kind: usize,
        lane: LaneId,
        at: (i32, i32),
    },
    StructureRemoved {
        kind: usize,
        lane: LaneId,
        at: (i32, i32),
        destroyed: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ActiveEffect {
    pub def: EffectDef,
    pub remaining: u32,
    pub source: Option<StructureId>,
}

#[derive(Debug, Clone)]
pub struct BuildJob {
    pub kind: usize,
    pub at: Cell,
}

#[derive(Debug, Clone)]
pub enum UnitMode {
    /// Walking the lane's flow field to an exit.
    FollowLane,
    Idle,
    Moving {
        path: Vec<(i32, i32)>,
        step: usize,
        dest: Cell,
        job: Option<BuildJob>,
    },
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub id: UnitId,
    pub kind: usize,
    pub owner: PlayerId,
    pub team: TeamId,
    pub lane: LaneId,
    pub x: f64,
    pub y: f64,
    pub hp: f64,
    pub max_hp: f64,
    pub effects: Vec<ActiveEffect>,
    pub mode: UnitMode,
    /// Path length left to the exit (flow field units), for targeting.
    pub remaining: f64,
    pub attack_cooldown: u32,
    pub attack_target: Option<StructureId>,
    pub facing: (f64, f64),
    pub killer: Option<Killer>,
    pub exited: bool,
    /// Build orders waiting behind the current one (builders queue).
    pub jobs: VecDeque<BuildJob>,
}

impl Unit {
    pub fn tile(&self) -> (i32, i32) {
        (self.x.floor() as i32, self.y.floor() as i32)
    }

    pub fn effect(&self, kind: EffectKind) -> Option<&ActiveEffect> {
        self.effects.iter().find(|e| e.def.kind == kind)
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0 && !self.exited
    }
}

#[derive(Debug, Clone)]
pub struct Structure {
    pub id: StructureId,
    pub kind: usize,
    pub owner: PlayerId,
    pub team: TeamId,
    pub lane: LaneId,
    /// Top-left tile of the footprint.
    pub x: i32,
    pub y: i32,
    pub hp: Option<f64>,
    pub max_hp: Option<f64>,
    /// Ticks until the next shot (fractional with haste).
    pub cooldown: f64,
    pub targeting: Targeting,
    /// Construction ticks left; the structure does nothing until 0.
    pub constructing: u32,
    pub construct_total: u32,
    /// Everything paid for it, for selling.
    pub invested: BTreeMap<String, i64>,
    pub buffs: Vec<ActiveEffect>,
    pub last_target: Option<UnitId>,
    pub killer: Option<UnitId>,
}

impl Structure {
    pub fn center(&self, size: u8) -> (f64, f64) {
        (
            self.x as f64 + size as f64 / 2.0,
            self.y as f64 + size as f64 / 2.0,
        )
    }

    pub fn cell(&self) -> Cell {
        Cell::new(self.lane, self.x, self.y)
    }

    fn buff(&self, kind: EffectKind) -> f64 {
        self.buffs
            .iter()
            .find(|e| e.def.kind == kind)
            .map(|e| {
                if kind == EffectKind::Armor {
                    e.def.amount
                } else {
                    e.def.percent
                }
            })
            .unwrap_or(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct Projectile {
    pub id: u32,
    pub source: StructureId,
    /// Structure kind index the stats come from.
    pub kind: usize,
    pub team: TeamId,
    pub lane: LaneId,
    pub x: f64,
    pub y: f64,
    pub target: UnitId,
    /// Tiles per tick.
    pub speed: f64,
    pub damage: f64,
}

#[derive(Debug, Clone)]
struct Timer {
    remaining: u32,
    period: Option<u32>,
}

/// Everything needed to start a game besides content.
#[derive(Debug, Clone)]
pub struct SimSetup {
    pub players: Vec<PlayerInfo>,
    pub lanes: Vec<LaneSpec>,
    pub seed: u64,
    /// Fraction of the invested cost refunded on sale when a structure kind
    /// doesn't set `sell_value`.
    pub default_sell_value: f64,
}

/// A player command as recorded for replays.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedCommand {
    pub tick: u64,
    pub player: PlayerId,
    pub command: Command,
}

pub struct Sim {
    pub content: Arc<Content>,
    pub tick: u64,
    pub players: Vec<PlayerInfo>,
    pub lanes: Vec<Lane>,
    pub units: BTreeMap<UnitId, Unit>,
    pub structures: BTreeMap<StructureId, Structure>,
    pub projectiles: Vec<Projectile>,
    resources: BTreeMap<Scope, BTreeMap<String, i64>>,
    timers: BTreeMap<(usize, u32), Timer>,
    allowed: BTreeMap<PlayerId, BTreeSet<usize>>,
    require_builder: BTreeSet<PlayerId>,
    tick_subscribers: BTreeSet<usize>,
    rng: Rng,
    next_unit: u32,
    next_structure: u32,
    next_projectile: u32,
    pending: Vec<(Source, Command)>,
    events: Vec<Routed>,
    default_sell_value: f64,
    result: Option<GameResult>,
    /// HUD messages for clients to drain.
    pub messages: Vec<(MessageTarget, String)>,
    /// Visual hints for clients to drain.
    pub fx: Vec<Fx>,
    /// Diagnostics (refused module commands …) for the host to drain.
    pub log: Vec<String>,
    /// Every player command, for replays.
    pub command_log: Vec<LoggedCommand>,
}

impl Sim {
    pub fn new(content: Arc<Content>, setup: SimSetup) -> Result<Sim, String> {
        if setup.players.is_empty() {
            return Err("a game needs at least one player".into());
        }
        for (i, p) in setup.players.iter().enumerate() {
            if p.id as usize != i {
                return Err("player ids must be 0, 1, 2, …".into());
            }
        }
        let tile_flags: Vec<TileFlags> = content.tiles.iter().map(|t| t.flags).collect();
        let mut masks: Vec<TileFlags> = Vec::new();
        for u in &content.units {
            if !u.moves_on.is_empty() && !masks.contains(&u.moves_on) {
                masks.push(u.moves_on);
            }
        }
        masks.sort();
        let mut lanes = Vec::new();
        for (i, spec) in setup.lanes.into_iter().enumerate() {
            if spec.tiles.len() != (spec.width * spec.height) as usize {
                return Err(format!("lane {i}: grid size mismatch"));
            }
            if spec.tiles.iter().any(|&t| t >= tile_flags.len()) {
                return Err(format!("lane {i}: unknown tile kind"));
            }
            lanes.push(Lane::new(i as LaneId, spec, &tile_flags, &masks));
        }
        Ok(Sim {
            content,
            tick: 0,
            players: setup.players,
            lanes,
            units: BTreeMap::new(),
            structures: BTreeMap::new(),
            projectiles: Vec::new(),
            resources: BTreeMap::new(),
            timers: BTreeMap::new(),
            allowed: BTreeMap::new(),
            require_builder: BTreeSet::new(),
            tick_subscribers: BTreeSet::new(),
            rng: Rng::new(setup.seed),
            next_unit: 1,
            next_structure: 1,
            next_projectile: 1,
            pending: Vec::new(),
            events: Vec::new(),
            default_sell_value: setup.default_sell_value,
            result: None,
            messages: Vec::new(),
            fx: Vec::new(),
            log: Vec::new(),
            command_log: Vec::new(),
        })
    }

    // ----- queries -------------------------------------------------------

    pub fn result(&self) -> Option<&GameResult> {
        self.result.as_ref()
    }

    pub fn team_of(&self, player: PlayerId) -> TeamId {
        if player == NEUTRAL {
            return NEUTRAL_TEAM;
        }
        self.players
            .get(player as usize)
            .map(|p| p.team)
            .unwrap_or(NEUTRAL_TEAM)
    }

    pub fn resource(&self, scope: Scope, name: &str) -> i64 {
        self.resources
            .get(&scope)
            .and_then(|m| m.get(name))
            .copied()
            .unwrap_or(0)
    }

    pub fn resources(&self, scope: Scope) -> impl Iterator<Item = (&str, i64)> {
        self.resources
            .get(&scope)
            .into_iter()
            .flat_map(|m| m.iter().map(|(k, v)| (k.as_str(), *v)))
    }

    /// The build palette of a player: `None` means everything.
    pub fn allowed(&self, player: PlayerId) -> Option<&BTreeSet<usize>> {
        self.allowed.get(&player)
    }

    pub fn requires_builder(&self, player: PlayerId) -> bool {
        self.require_builder.contains(&player)
    }

    pub fn can_build_on(&self, player: PlayerId, lane: LaneId) -> bool {
        match self.lanes.get(lane as usize).map(|l| l.owner) {
            Some(LaneOwner::Player(p)) => p == player,
            Some(LaneOwner::Team(t)) => t == self.team_of(player),
            Some(LaneOwner::Shared) => true,
            None => false,
        }
    }

    pub fn lane_info(&self) -> Vec<LaneInfo> {
        self.lanes
            .iter()
            .map(|l| LaneInfo {
                id: l.id,
                owner: l.owner,
                width: l.width,
                height: l.height,
                spawns: l
                    .spawns
                    .iter()
                    .map(|&(x, y)| Cell::new(l.id, x, y))
                    .collect(),
                exits: l
                    .exits
                    .iter()
                    .map(|&(x, y)| Cell::new(l.id, x, y))
                    .collect(),
            })
            .collect()
    }

    pub fn structure_at(&self, cell: Cell) -> Option<&Structure> {
        let lane = self.lanes.get(cell.lane as usize)?;
        if !lane.in_bounds(cell.x, cell.y) {
            return None;
        }
        lane.occupant[lane.idx(cell.x, cell.y)].and_then(|id| self.structures.get(&id))
    }

    /// Whether `kind` could be placed at `at` by `owner` right now
    /// (ignoring cost). Used by clients to preview placement.
    pub fn check_placement(
        &mut self,
        kind: usize,
        at: Cell,
        owner: PlayerId,
    ) -> Result<(), String> {
        self.validate_site(kind, at, owner)?;
        let size = self.content.structures[kind].size;
        let blocks = self.content.structures[kind].blocks;
        let cells: Vec<(i32, i32)> = Lane::footprint(at.x, at.y, size).collect();
        let r = self.block_and_check(at.lane, &cells, blocks);
        if r.is_ok() {
            self.set_blocks(at.lane, &cells, TileFlags::NONE);
        }
        r
    }

    pub fn sell_refund(&self, s: &Structure) -> BTreeMap<String, i64> {
        let frac = self.content.structures[s.kind]
            .sell_value
            .unwrap_or(self.default_sell_value);
        s.invested
            .iter()
            .map(|(k, v)| (k.clone(), (*v as f64 * frac).floor() as i64))
            .filter(|(_, v)| *v > 0)
            .collect()
    }

    /// A hash of the whole simulation state, for replay verification and
    /// desync detection.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick);
        h.u64(self.rng.state());
        for u in self.units.values() {
            h.u64(u.id.0 as u64);
            h.u64(u.kind as u64);
            h.u64(u.x.to_bits());
            h.u64(u.y.to_bits());
            h.u64(u.hp.to_bits());
        }
        for s in self.structures.values() {
            h.u64(s.id.0 as u64);
            h.u64(s.kind as u64);
            h.u64(((s.x as u64) << 32) | s.y as u32 as u64);
            h.u64(s.hp.map(f64::to_bits).unwrap_or(0));
            h.u64(s.cooldown.to_bits());
        }
        for p in &self.projectiles {
            h.u64(p.x.to_bits());
            h.u64(p.y.to_bits());
        }
        for (scope, m) in &self.resources {
            h.bytes(format!("{scope:?}").as_bytes());
            for (k, v) in m {
                h.bytes(k.as_bytes());
                h.u64(*v as u64);
            }
        }
        h.0
    }

    // ----- input ---------------------------------------------------------

    /// Queues a command for the next tick.
    pub fn queue(&mut self, source: Source, command: Command) {
        if let Source::Player(p) = source {
            self.command_log.push(LoggedCommand {
                tick: self.tick,
                player: p,
                command: command.clone(),
            });
        }
        self.pending.push((source, command));
    }

    pub fn player_left(&mut self, player: PlayerId) {
        self.emit(Event::PlayerLeft { player });
    }

    // ----- the tick ------------------------------------------------------

    pub fn step(&mut self) -> Vec<Routed> {
        if self.result.is_some() {
            return Vec::new();
        }
        if self.tick == 0 {
            let start = Event::GameStart {
                players: self.players.clone(),
                lanes: self.lane_info(),
            };
            self.events.insert(
                0,
                Routed {
                    audience: Audience::All,
                    event: start,
                },
            );
        }

        let pending = std::mem::take(&mut self.pending);
        for (source, command) in pending {
            if self.result.is_some() {
                break;
            }
            if let Err(e) = self.apply(source, command) {
                self.refuse(source, e);
            }
        }

        self.run_timers();
        self.run_construction();
        self.move_units();
        self.run_effects();
        self.run_auras();
        self.structures_attack();
        self.move_projectiles();
        self.units_attack();
        self.resolve();

        if !self.tick_subscribers.is_empty() {
            self.events.push(Routed {
                audience: Audience::TickSubscribers,
                event: Event::Tick { n: self.tick },
            });
        }
        self.tick += 1;
        std::mem::take(&mut self.events)
    }

    pub fn is_tick_subscriber(&self, module: usize) -> bool {
        self.tick_subscribers.contains(&module)
    }

    fn emit(&mut self, event: Event) {
        self.events.push(Routed {
            audience: Audience::All,
            event,
        });
    }

    fn refuse(&mut self, source: Source, reason: String) {
        match source {
            Source::Player(p) => self.messages.push((MessageTarget::Player(p), reason)),
            Source::Module(m) => self.log.push(format!("module #{m}: {reason}")),
            Source::System => self.log.push(reason),
        }
    }

    // ----- commands ------------------------------------------------------

    fn apply(&mut self, source: Source, command: Command) -> Result<(), String> {
        let player = match source {
            Source::Player(p) => Some(p),
            _ => None,
        };
        match command {
            Command::Spawn {
                kind,
                lane,
                owner,
                hp_percent,
                at,
                idle,
            } => {
                if player.is_some() {
                    return Err("players can't spawn units".into());
                }
                let k = self
                    .content
                    .unit(&kind)
                    .ok_or_else(|| format!("unknown unit `{kind}`"))?;
                self.spawn_unit(k, lane, owner, hp_percent, at, idle)
                    .map(|_| ())
            }
            Command::OrderMove { unit, to } => {
                let u = self.units.get(&unit).ok_or("no such unit")?;
                if player.is_some_and(|p| p != u.owner) {
                    return Err("not your unit".into());
                }
                self.order_move(unit, to, None)?;
                self.units.get_mut(&unit).unwrap().jobs.clear();
                Ok(())
            }
            Command::Build {
                kind,
                at,
                owner,
                builder,
            } => {
                let k = self
                    .content
                    .structure(&kind)
                    .ok_or_else(|| format!("unknown structure `{kind}`"))?;
                if let Some(p) = player {
                    if p != owner {
                        return Err("you can only build for yourself".into());
                    }
                    if self.allowed.get(&p).is_some_and(|a| !a.contains(&k)) {
                        return Err(format!("you can't build {}", self.structure_name(k)));
                    }
                    if builder.is_none() && self.require_builder.contains(&p) {
                        return Err("select a builder first".into());
                    }
                }
                match builder {
                    Some(b) => self.order_build(b, k, at, owner),
                    None => self.place_structure(k, at, owner).map(|_| ()),
                }
            }
            Command::Sell { structure } => {
                let s = self.structures.get(&structure).ok_or("no such structure")?;
                if player.is_some_and(|p| p != s.owner) {
                    return Err("not your structure".into());
                }
                self.sell(structure);
                Ok(())
            }
            Command::Upgrade { structure, to } => {
                let s = self.structures.get(&structure).ok_or("no such structure")?;
                if player.is_some_and(|p| p != s.owner) {
                    return Err("not your structure".into());
                }
                let to = self
                    .content
                    .structure(&to)
                    .ok_or_else(|| format!("unknown structure `{to}`"))?;
                if let Some(p) = player
                    && self.allowed.get(&p).is_some_and(|a| !a.contains(&to))
                {
                    return Err(format!("you can't build {}", self.structure_name(to)));
                }
                self.upgrade(structure, to)
            }
            Command::SetTargeting { structure, mode } => {
                let s = self
                    .structures
                    .get_mut(&structure)
                    .ok_or("no such structure")?;
                if player.is_some_and(|p| p != s.owner) {
                    return Err("not your structure".into());
                }
                s.targeting = mode;
                Ok(())
            }
            Command::Request { request, data } => {
                let Some(p) = player else {
                    return Err("only players send requests".into());
                };
                self.emit(Event::PlayerRequest {
                    player: p,
                    request,
                    data,
                });
                Ok(())
            }
            _ if player.is_some() => Err("players can't issue that command".into()),
            Command::AddResource {
                scope,
                name,
                delta,
                allow_negative,
            } => self.add_resource(scope, &name, delta, allow_negative),
            Command::Purchase {
                scope,
                cost,
                then,
                notify,
            } => {
                if !self.can_afford(scope, &cost) {
                    if let Some(p) = notify {
                        let text = format!("not enough {}", self.shortfall(scope, &cost));
                        self.messages.push((MessageTarget::Player(p), text));
                    }
                    return Ok(());
                }
                self.pay(scope, &cost);
                for c in then {
                    if let Err(e) = self.apply(source, c) {
                        self.refuse(source, e);
                    }
                }
                Ok(())
            }
            Command::SetTimer { id, ticks, repeat } => {
                let Source::Module(m) = source else {
                    return Err("timers belong to modules".into());
                };
                let ticks = ticks.max(1);
                self.timers.insert(
                    (m, id),
                    Timer {
                        remaining: ticks,
                        period: repeat.then_some(ticks),
                    },
                );
                Ok(())
            }
            Command::CancelTimer { id } => {
                if let Source::Module(m) = source {
                    self.timers.remove(&(m, id));
                }
                Ok(())
            }
            Command::SetAllowed { player, structures } => {
                match structures {
                    None => {
                        self.allowed.remove(&player);
                    }
                    Some(ids) => {
                        let mut set = BTreeSet::new();
                        for id in ids {
                            set.insert(
                                self.content
                                    .structure(&id)
                                    .ok_or_else(|| format!("unknown structure `{id}`"))?,
                            );
                        }
                        self.allowed.insert(player, set);
                    }
                }
                Ok(())
            }
            Command::SetRequireBuilder { player, required } => {
                if required {
                    self.require_builder.insert(player);
                } else {
                    self.require_builder.remove(&player);
                }
                Ok(())
            }
            Command::SetTickEvents { enabled } => {
                if let Source::Module(m) = source {
                    if enabled {
                        self.tick_subscribers.insert(m);
                    } else {
                        self.tick_subscribers.remove(&m);
                    }
                }
                Ok(())
            }
            Command::Message { to, text } => {
                self.messages.push((to, text));
                Ok(())
            }
            Command::Signal { name, data } => {
                self.emit(Event::Signal { name, data });
                Ok(())
            }
            Command::EndGame { result } => {
                if self.result.is_none() {
                    self.result = Some(result);
                }
                Ok(())
            }
        }
    }

    fn structure_name(&self, kind: usize) -> String {
        let k = &self.content.structures[kind];
        k.name.clone().unwrap_or_else(|| k.id.clone())
    }

    // ----- resources -----------------------------------------------------

    fn add_resource(
        &mut self,
        scope: Scope,
        name: &str,
        delta: i64,
        allow_negative: bool,
    ) -> Result<(), String> {
        let value = self.resource(scope, name).saturating_add(delta);
        if value < 0 && !allow_negative {
            return Err(format!("{name} would go negative"));
        }
        self.resources
            .entry(scope)
            .or_default()
            .insert(name.to_string(), value);
        if delta != 0 {
            self.emit(Event::ResourceChanged {
                scope,
                name: name.to_string(),
                value,
                delta,
            });
        }
        Ok(())
    }

    pub fn can_afford(&self, scope: Scope, cost: &BTreeMap<String, i64>) -> bool {
        cost.iter().all(|(k, v)| self.resource(scope, k) >= *v)
    }

    fn shortfall(&self, scope: Scope, cost: &BTreeMap<String, i64>) -> String {
        cost.iter()
            .filter(|(k, v)| self.resource(scope, k) < **v)
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn pay(&mut self, scope: Scope, cost: &BTreeMap<String, i64>) {
        for (k, v) in cost {
            let _ = self.add_resource(scope, k, -*v, true);
        }
    }

    // ----- units ---------------------------------------------------------

    fn spawn_unit(
        &mut self,
        kind: usize,
        lane: LaneId,
        owner: PlayerId,
        hp_percent: u32,
        at: Option<Cell>,
        idle: bool,
    ) -> Result<UnitId, String> {
        let l = self.lanes.get(lane as usize).ok_or("no such lane")?;
        let k = &self.content.units[kind];
        let (tx, ty) = match at {
            Some(c) => {
                if c.lane != lane || !l.in_bounds(c.x, c.y) {
                    return Err("spawn cell outside the lane".into());
                }
                (c.x, c.y)
            }
            None => {
                if l.spawns.is_empty() {
                    return Err("lane has no spawn".into());
                }
                let n = l.spawns.len() as u64;
                l.spawns[self.rng.below(n) as usize]
            }
        };
        if !idle && l.dist(k.moves_on, tx, ty) == UNREACHABLE {
            return Err(format!("`{}` has no route on lane {lane}", k.id));
        }
        let max_hp = k.hp * hp_percent as f64 / 100.0;
        let id = UnitId(self.next_unit);
        self.next_unit += 1;
        let team = self.team_of(owner);
        self.units.insert(
            id,
            Unit {
                id,
                kind,
                owner,
                team,
                lane,
                x: tx as f64 + 0.5,
                y: ty as f64 + 0.5,
                hp: max_hp.max(1.0),
                max_hp: max_hp.max(1.0),
                effects: Vec::new(),
                mode: if idle {
                    UnitMode::Idle
                } else {
                    UnitMode::FollowLane
                },
                remaining: f64::MAX,
                attack_cooldown: 0,
                attack_target: None,
                jobs: VecDeque::new(),
                facing: (1.0, 0.0),
                killer: None,
                exited: false,
            },
        );
        let kind = k.id.clone();
        self.emit(Event::UnitSpawned {
            unit: id,
            kind,
            owner,
            lane,
        });
        Ok(id)
    }

    fn order_move(&mut self, unit: UnitId, to: Cell, job: Option<BuildJob>) -> Result<(), String> {
        let u = self.units.get(&unit).ok_or("no such unit")?;
        if u.lane != to.lane {
            return Err("that is on another lane".into());
        }
        let mask = self.content.units[u.kind].moves_on;
        let lane = &self.lanes[u.lane as usize];
        let from = u.tile();
        let path = lane
            .path_to(mask, from, (to.x, to.y))
            .ok_or("can't get there")?;
        let u = self.units.get_mut(&unit).unwrap();
        u.mode = UnitMode::Moving {
            path,
            step: 0,
            dest: to,
            job,
        };
        Ok(())
    }

    // ----- structures ----------------------------------------------------

    /// Bounds, ownership and buildable tiles; not routes or cost.
    fn validate_site(&self, kind: usize, at: Cell, owner: PlayerId) -> Result<(), String> {
        if owner == NEUTRAL || owner as usize >= self.players.len() {
            return Err("structures need a player owner".into());
        }
        let lane = self.lanes.get(at.lane as usize).ok_or("no such lane")?;
        if !self.can_build_on(owner, at.lane) {
            return Err("you can't build on this lane".into());
        }
        let size = self.content.structures[kind].size;
        for (x, y) in Lane::footprint(at.x, at.y, size) {
            if !lane.in_bounds(x, y) {
                return Err("out of bounds".into());
            }
            let i = lane.idx(x, y);
            if lane.occupant[i].is_some() {
                return Err("something is already built there".into());
            }
            if !lane.base[i].contains(TileFlags::BUILD) {
                return Err("can't build there".into());
            }
        }
        Ok(())
    }

    fn set_blocks(&mut self, lane: LaneId, cells: &[(i32, i32)], blocks: TileFlags) {
        let l = &mut self.lanes[lane as usize];
        let mut changed = blocks;
        for &(x, y) in cells {
            let i = l.idx(x, y);
            changed = changed | l.blocked[i];
            l.blocked[i] = blocks;
        }
        l.recompute(changed);
    }

    /// Applies `blocks` to `cells` and checks every route the lane must keep.
    /// On failure the change is undone.
    fn block_and_check(
        &mut self,
        lane: LaneId,
        cells: &[(i32, i32)],
        blocks: TileFlags,
    ) -> Result<(), String> {
        let previous: Vec<TileFlags> = {
            let l = &self.lanes[lane as usize];
            cells.iter().map(|&(x, y)| l.blocked[l.idx(x, y)]).collect()
        };
        self.set_blocks(lane, cells, blocks);
        let l = &self.lanes[lane as usize];
        let mut err = None;
        for f in &l.fields {
            if f.guarded
                && l.spawns
                    .iter()
                    .any(|&(x, y)| f.dist[l.idx(x, y)] == UNREACHABLE)
            {
                err = Some("that would block the path");
                break;
            }
        }
        if err.is_none() {
            for u in self.units.values() {
                if u.lane != lane || !u.alive() || !matches!(u.mode, UnitMode::FollowLane) {
                    continue;
                }
                let mask = self.content.units[u.kind].moves_on;
                let (x, y) = u.tile();
                if l.in_bounds(x, y) && l.dist(mask, x, y) == UNREACHABLE {
                    err = Some(if cells.contains(&(x, y)) {
                        "a unit is in the way"
                    } else {
                        "that would trap a unit"
                    });
                    break;
                }
            }
        }
        if let Some(e) = err {
            let l = &mut self.lanes[lane as usize];
            let mut changed = blocks;
            for (&(x, y), prev) in cells.iter().zip(previous) {
                let i = l.idx(x, y);
                l.blocked[i] = prev;
                changed = changed | prev;
            }
            l.recompute(changed);
            return Err(e.into());
        }
        Ok(())
    }

    fn place_structure(
        &mut self,
        kind: usize,
        at: Cell,
        owner: PlayerId,
    ) -> Result<StructureId, String> {
        self.validate_site(kind, at, owner)?;
        let k = &self.content.structures[kind];
        let cost = k.cost.clone();
        let scope = Scope::Player(owner);
        if !self.can_afford(scope, &cost) {
            return Err(format!("not enough {}", self.shortfall(scope, &cost)));
        }
        let size = k.size;
        let blocks = k.blocks;
        let cells: Vec<(i32, i32)> = Lane::footprint(at.x, at.y, size).collect();
        self.block_and_check(at.lane, &cells, blocks)?;
        self.pay(scope, &cost);

        let k = &self.content.structures[kind];
        let id = StructureId(self.next_structure);
        self.next_structure += 1;
        let build = secs_to_ticks(k.build_time, 0);
        let structure = Structure {
            id,
            kind,
            owner,
            team: self.team_of(owner),
            lane: at.lane,
            x: at.x,
            y: at.y,
            hp: k.hp,
            max_hp: k.hp,
            cooldown: 0.0,
            targeting: k.targeting,
            constructing: build,
            construct_total: build,
            invested: cost,
            buffs: Vec::new(),
            last_target: None,
            killer: None,
        };
        let l = &mut self.lanes[at.lane as usize];
        for &(x, y) in &cells {
            let i = l.idx(x, y);
            l.occupant[i] = Some(id);
        }
        self.structures.insert(id, structure);
        self.fx.push(Fx::StructurePlaced {
            kind,
            lane: at.lane,
            at: (at.x, at.y),
        });
        if build == 0 {
            self.emit_built(id);
        }
        Ok(id)
    }

    fn emit_built(&mut self, id: StructureId) {
        let s = &self.structures[&id];
        let event = Event::StructureBuilt {
            structure: id,
            kind: self.content.structures[s.kind].id.clone(),
            owner: s.owner,
            at: s.cell(),
        };
        self.emit(event);
    }

    fn order_build(
        &mut self,
        builder: UnitId,
        kind: usize,
        at: Cell,
        owner: PlayerId,
    ) -> Result<(), String> {
        let u = self.units.get(&builder).ok_or("no such builder")?;
        if u.owner != owner {
            return Err("not your builder".into());
        }
        // Fail early, with the reason, rather than after the walk.
        self.check_placement(kind, at, owner)?;
        let cost = &self.content.structures[kind].cost;
        let scope = Scope::Player(owner);
        if !self.can_afford(scope, cost) {
            return Err(format!("not enough {}", self.shortfall(scope, cost)));
        }
        let u = self.units.get_mut(&builder).unwrap();
        // A busy builder queues the order.
        if let UnitMode::Moving { job: Some(_), .. } = u.mode {
            if u.jobs.iter().any(|j| j.at == at) {
                return Err("already queued there".into());
            }
            u.jobs.push_back(BuildJob { kind, at });
            return Ok(());
        }
        if u.tile() == (at.x, at.y) {
            self.place_structure(kind, at, owner)?;
            return Ok(());
        }
        self.order_move(builder, at, Some(BuildJob { kind, at }))
    }

    /// Starts the next queued build order of a builder that just finished
    /// one; orders that became impossible are dropped with a message.
    fn next_job(&mut self, builder: UnitId) {
        let owner = self.units[&builder].owner;
        while let Some(job) = self
            .units
            .get_mut(&builder)
            .and_then(|u| u.jobs.pop_front())
        {
            let result = self.check_placement(job.kind, job.at, owner).and_then(|_| {
                if self.units[&builder].tile() == (job.at.x, job.at.y) {
                    self.place_structure(job.kind, job.at, owner).map(|_| ())
                } else {
                    self.order_move(builder, job.at, Some(job.clone()))
                }
            });
            match result {
                Ok(()) if matches!(self.units[&builder].mode, UnitMode::Moving { .. }) => return,
                Ok(()) => {}
                Err(e) => self.messages.push((MessageTarget::Player(owner), e)),
            }
        }
    }

    fn remove_structure(&mut self, id: StructureId) -> Option<Structure> {
        let s = self.structures.remove(&id)?;
        let size = self.content.structures[s.kind].size;
        let cells: Vec<(i32, i32)> = Lane::footprint(s.x, s.y, size).collect();
        let l = &mut self.lanes[s.lane as usize];
        for &(x, y) in &cells {
            let i = l.idx(x, y);
            l.occupant[i] = None;
        }
        self.set_blocks(s.lane, &cells, TileFlags::NONE);
        Some(s)
    }

    fn sell(&mut self, id: StructureId) {
        let Some(s) = self.structures.get(&id) else {
            return;
        };
        let refund = self.sell_refund(s);
        let owner = s.owner;
        let s = self.remove_structure(id).unwrap();
        self.fx.push(Fx::StructureRemoved {
            kind: s.kind,
            lane: s.lane,
            at: (s.x, s.y),
            destroyed: false,
        });
        for (k, v) in refund {
            let _ = self.add_resource(Scope::Player(owner), &k, v, true);
        }
        self.emit(Event::StructureSold {
            structure: id,
            owner,
        });
    }

    fn upgrade(&mut self, id: StructureId, to: usize) -> Result<(), String> {
        let s = &self.structures[&id];
        let from = &self.content.structures[s.kind];
        if s.constructing > 0 {
            return Err("still under construction".into());
        }
        if !from
            .upgrades_to
            .iter()
            .any(|u| self.content.structure(u) == Some(to))
        {
            return Err(format!(
                "{} doesn't upgrade to that",
                self.structure_name(s.kind)
            ));
        }
        let new = &self.content.structures[to];
        if new.size != from.size {
            return Err("upgrades must keep the footprint size".into());
        }
        let scope = Scope::Player(s.owner);
        let cost = new.cost.clone();
        if !self.can_afford(scope, &cost) {
            return Err(format!("not enough {}", self.shortfall(scope, &cost)));
        }
        if new.blocks != from.blocks {
            let cells: Vec<(i32, i32)> = Lane::footprint(s.x, s.y, new.size).collect();
            let lane = s.lane;
            self.block_and_check(lane, &cells, new.blocks)?;
        }
        self.pay(scope, &cost);
        let new = &self.content.structures[to];
        let build = secs_to_ticks(new.build_time, 0);
        let s = self.structures.get_mut(&id).unwrap();
        s.kind = to;
        s.hp = new.hp;
        s.max_hp = new.hp;
        s.targeting = new.targeting;
        s.constructing = build;
        s.construct_total = build;
        s.cooldown = 0.0;
        for (k, v) in cost {
            *s.invested.entry(k).or_default() += v;
        }
        self.fx.push(Fx::StructurePlaced {
            kind: to,
            lane: s.lane,
            at: (s.x, s.y),
        });
        if build == 0 {
            self.emit_built(id);
        }
        Ok(())
    }

    // ----- phases --------------------------------------------------------

    fn run_timers(&mut self) {
        let mut fired = Vec::new();
        for (&key, t) in self.timers.iter_mut() {
            t.remaining -= 1;
            if t.remaining == 0 {
                fired.push(key);
            }
        }
        for key in fired {
            let t = self.timers.get_mut(&key).unwrap();
            match t.period {
                Some(p) => t.remaining = p,
                None => {
                    self.timers.remove(&key);
                }
            }
            self.events.push(Routed {
                audience: Audience::Module(key.0),
                event: Event::Timer { id: key.1 },
            });
        }
    }

    fn run_construction(&mut self) {
        let mut done = Vec::new();
        for s in self.structures.values_mut() {
            if s.constructing > 0 {
                s.constructing -= 1;
                if s.constructing == 0 {
                    done.push(s.id);
                }
            }
        }
        for id in done {
            self.emit_built(id);
        }
    }

    fn unit_speed(&self, u: &Unit) -> f64 {
        if u.effect(EffectKind::Stun).is_some() {
            return 0.0;
        }
        let slow = u
            .effect(EffectKind::Slow)
            .map(|e| e.def.percent)
            .unwrap_or(0.0);
        let factor = (1.0 - slow / 100.0).clamp(0.0, 1.0);
        self.content.units[u.kind].speed * factor / TICK_RATE as f64
    }

    /// The nearest hostile structure with hp within a unit's attack range.
    fn unit_attack_target(&self, u: &Unit) -> Option<StructureId> {
        let attack = self.content.units[u.kind].attack.as_ref()?;
        let mut best: Option<(f64, StructureId)> = None;
        for s in self.structures.values() {
            if s.lane != u.lane || s.team == u.team || s.hp.is_none_or(|hp| hp <= 0.0) {
                continue;
            }
            let k = &self.content.structures[s.kind];
            if let Some(mask) = attack.targets
                && !mask.intersects(k.blocks)
            {
                continue;
            }
            let (cx, cy) = s.center(k.size);
            let d = dist(u.x, u.y, cx, cy) - k.size as f64 / 2.0;
            if d <= attack.range && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, s.id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn move_units(&mut self) {
        let ids: Vec<UnitId> = self.units.keys().copied().collect();
        let mut arrived = Vec::new();
        for id in ids {
            let u = &self.units[&id];
            if !u.alive() {
                continue;
            }
            let target = if u.effect(EffectKind::Stun).is_none() {
                self.unit_attack_target(u)
            } else {
                None
            };
            let speed = self.unit_speed(u);
            let mask = self.content.units[u.kind].moves_on;
            let lane = &self.lanes[u.lane as usize];
            let mut u = u.clone();
            u.attack_target = target;
            if target.is_none() && speed > 0.0 {
                match &mut u.mode {
                    UnitMode::Idle => {}
                    UnitMode::FollowLane => {
                        let field = lane.field(mask).expect("flow field for every unit mask");
                        let mut budget = speed;
                        for _ in 0..4 {
                            let (tx, ty) = u.tile();
                            if lane.is_exit(tx, ty) {
                                u.exited = true;
                                break;
                            }
                            let Some((nx, ny)) = lane.next_step(&field.dist, mask, tx, ty) else {
                                break;
                            };
                            budget = step_toward(
                                &mut u.x,
                                &mut u.y,
                                &mut u.facing,
                                nx as f64 + 0.5,
                                ny as f64 + 0.5,
                                budget,
                            );
                            if budget <= 1e-9 {
                                break;
                            }
                        }
                        let (tx, ty) = u.tile();
                        if lane.is_exit(tx, ty) {
                            u.exited = true;
                        }
                    }
                    UnitMode::Moving {
                        path,
                        step,
                        dest,
                        job,
                    } => {
                        let mut budget = speed;
                        let mut blocked = false;
                        while *step < path.len() && budget > 1e-9 {
                            let (nx, ny) = path[*step];
                            if !lane.passable(nx, ny, mask) {
                                blocked = true;
                                break;
                            }
                            budget = step_toward(
                                &mut u.x,
                                &mut u.y,
                                &mut u.facing,
                                nx as f64 + 0.5,
                                ny as f64 + 0.5,
                                budget,
                            );
                            if (u.x - (nx as f64 + 0.5)).abs() < 1e-9
                                && (u.y - (ny as f64 + 0.5)).abs() < 1e-9
                            {
                                *step += 1;
                            }
                        }
                        if blocked {
                            // Re-plan around whatever appeared.
                            let (dest, job) = (*dest, job.take());
                            let here = (u.x.floor() as i32, u.y.floor() as i32);
                            match lane.path_to(mask, here, (dest.x, dest.y)) {
                                Some(p) => {
                                    u.mode = UnitMode::Moving {
                                        path: p,
                                        step: 0,
                                        dest,
                                        job,
                                    }
                                }
                                None => u.mode = UnitMode::Idle,
                            }
                        } else if *step >= path.len() {
                            arrived.push((id, *dest, job.take()));
                            u.mode = UnitMode::Idle;
                        }
                    }
                }
            }
            if matches!(u.mode, UnitMode::FollowLane) {
                let (tx, ty) = u.tile();
                let d = lane.dist(mask, tx, ty);
                u.remaining = if d == UNREACHABLE { f64::MAX } else { d as f64 };
            }
            self.units.insert(id, u);
        }
        for (id, at, job) in arrived {
            self.emit(Event::UnitArrived { unit: id, at });
            if let Some(job) = job {
                let owner = self.units[&id].owner;
                if let Err(e) = self.place_structure(job.kind, job.at, owner) {
                    self.messages.push((MessageTarget::Player(owner), e));
                }
                self.next_job(id);
            }
        }
    }

    fn run_effects(&mut self) {
        for u in self.units.values_mut() {
            if !u.alive() {
                continue;
            }
            for e in &u.effects {
                if e.def.kind == EffectKind::Dot && u.hp > 0.0 {
                    u.hp -= e.def.dps / TICK_RATE as f64;
                    if u.hp <= 0.0 && u.killer.is_none() {
                        u.killer = e.source.map(Killer::Structure);
                    }
                }
            }
            for e in &mut u.effects {
                e.remaining = e.remaining.saturating_sub(1);
            }
            u.effects.retain(|e| e.remaining > 0);
        }
        for s in self.structures.values_mut() {
            for e in &mut s.buffs {
                e.remaining = e.remaining.saturating_sub(1);
            }
            s.buffs.retain(|e| e.remaining > 0);
        }
    }

    fn run_auras(&mut self) {
        let mut unit_effects: Vec<(UnitId, EffectDef, StructureId)> = Vec::new();
        let mut structure_effects: Vec<(StructureId, EffectDef)> = Vec::new();
        for s in self.structures.values() {
            let k = &self.content.structures[s.kind];
            let Some(aura) = &k.aura else { continue };
            if s.constructing > 0 {
                continue;
            }
            let (cx, cy) = s.center(k.size);
            match aura.filter {
                AuraFilter::Enemies => {
                    for u in self.units.values() {
                        if u.lane == s.lane
                            && u.team != s.team
                            && u.alive()
                            && dist(u.x, u.y, cx, cy) <= aura.radius
                        {
                            for e in &aura.effects {
                                unit_effects.push((u.id, e.clone(), s.id));
                            }
                        }
                    }
                }
                AuraFilter::Allies => {
                    for o in self.structures.values() {
                        let ok = &self.content.structures[o.kind];
                        let (ox, oy) = o.center(ok.size);
                        if o.lane == s.lane
                            && o.team == s.team
                            && dist(ox, oy, cx, cy) <= aura.radius
                        {
                            for e in &aura.effects {
                                structure_effects.push((o.id, e.clone()));
                            }
                        }
                    }
                }
            }
        }
        for (u, mut e, src) in unit_effects {
            // Auras hold while in range; a short duration keeps them alive
            // between ticks.
            e.duration = e.duration.max(2.0 / TICK_RATE as f64);
            self.apply_effect(u, &e, Some(src));
        }
        for (s, mut e) in structure_effects {
            e.duration = e.duration.max(2.0 / TICK_RATE as f64);
            if let Some(s) = self.structures.get_mut(&s) {
                add_effect(&mut s.buffs, &e, None);
            }
        }
    }

    fn apply_effect(&mut self, unit: UnitId, def: &EffectDef, source: Option<StructureId>) {
        let Some(u) = self.units.get_mut(&unit) else {
            return;
        };
        if def.kind == EffectKind::Bounce
            || self.content.units[u.kind].immune_to.contains(&def.kind)
        {
            return;
        }
        add_effect(&mut u.effects, def, source);
    }

    fn pick_target(&self, s: &Structure) -> Option<UnitId> {
        let k = &self.content.structures[s.kind];
        let (cx, cy) = s.center(k.size);
        let mut best: Option<(&Unit, f64)> = None;
        for u in self.units.values() {
            if u.lane != s.lane || u.team == s.team || !u.alive() {
                continue;
            }
            let uk = &self.content.units[u.kind];
            if !uk.moves_on.intersects(k.targets) {
                continue;
            }
            let d = dist(u.x, u.y, cx, cy);
            if d > k.range + uk.size.radius() {
                continue;
            }
            let better = match best {
                None => true,
                Some((b, bd)) => match s.targeting {
                    Targeting::First => u.remaining < b.remaining,
                    Targeting::Last => u.remaining > b.remaining,
                    Targeting::Nearest => d < bd,
                    Targeting::Strongest => u.hp > b.hp,
                    Targeting::Weakest => u.hp < b.hp,
                },
            };
            if better {
                best = Some((u, d));
            }
        }
        best.map(|(u, _)| u.id)
    }

    fn structures_attack(&mut self) {
        let content = Arc::clone(&self.content);
        let ids: Vec<StructureId> = self.structures.keys().copied().collect();
        for id in ids {
            let s = &self.structures[&id];
            let k = &content.structures[s.kind];
            if s.constructing > 0 || !k.can_attack() {
                continue;
            }
            let haste = s.buff(EffectKind::Haste);
            let cooldown = s.cooldown - (1.0 + haste / 100.0);
            if cooldown > 0.0 {
                self.structures.get_mut(&id).unwrap().cooldown = cooldown;
                continue;
            }
            let Some(target) = self.pick_target(s) else {
                let s = self.structures.get_mut(&id).unwrap();
                s.cooldown = 0.0;
                s.last_target = None;
                continue;
            };
            let damage = k.damage * (1.0 + s.buff(EffectKind::Damage) / 100.0);
            let (cx, cy) = s.center(k.size);
            let (kind, lane, team) = (s.kind, s.lane, s.team);
            let t = &self.units[&target];
            let (tx, ty) = (t.x, t.y);
            self.fx.push(Fx::Attack {
                structure: id,
                kind,
                lane,
                from: (cx, cy),
                to: (tx, ty),
            });
            match k.projectile {
                Some(speed) if speed > 0.0 => {
                    let pid = self.next_projectile;
                    self.next_projectile += 1;
                    self.projectiles.push(Projectile {
                        id: pid,
                        source: id,
                        kind,
                        team,
                        lane,
                        x: cx,
                        y: cy,
                        target,
                        speed: speed / TICK_RATE as f64,
                        damage,
                    });
                }
                _ => self.hit(id, kind, team, target, damage),
            }
            let cd = secs_to_ticks(k.cooldown, 1) as f64;
            let s = self.structures.get_mut(&id).unwrap();
            s.cooldown = cooldown + cd;
            s.last_target = Some(target);
        }
    }

    fn move_projectiles(&mut self) {
        let mut hits = Vec::new();
        let units = &self.units;
        self.projectiles.retain_mut(|p| {
            let Some(t) = units.get(&p.target).filter(|t| t.alive()) else {
                return false; // wasted
            };
            let d = dist(p.x, p.y, t.x, t.y);
            if d <= p.speed {
                hits.push((p.source, p.kind, p.team, p.target, p.damage));
                return false;
            }
            p.x += (t.x - p.x) / d * p.speed;
            p.y += (t.y - p.y) / d * p.speed;
            true
        });
        for (source, kind, team, target, damage) in hits {
            self.hit(source, kind, team, target, damage);
        }
    }

    /// Damage after the damage table and armor.
    fn mitigate(&self, damage: f64, damage_type: &str, u: &Unit) -> f64 {
        let k = &self.content.units[u.kind];
        let armor = k.armor
            + u.effect(EffectKind::Armor)
                .map(|e| e.def.amount)
                .unwrap_or(0.0);
        damage * self.content.damage.multiplier(damage_type, &k.armor_type) * armor_factor(armor)
    }

    fn damage_unit(&mut self, unit: UnitId, amount: f64, damage_type: &str, source: StructureId) {
        let Some(u) = self.units.get(&unit) else {
            return;
        };
        if !u.alive() {
            return;
        }
        let dealt = self.mitigate(amount, damage_type, u);
        let u = self.units.get_mut(&unit).unwrap();
        u.hp -= dealt;
        if u.hp <= 0.0 && u.killer.is_none() {
            u.killer = Some(Killer::Structure(source));
        }
    }

    fn hit(&mut self, source: StructureId, kind: usize, team: TeamId, target: UnitId, damage: f64) {
        let content = Arc::clone(&self.content);
        let k = &content.structures[kind];
        let Some(t) = self.units.get(&target) else {
            return;
        };
        let (lane, tx, ty) = (t.lane, t.x, t.y);
        self.fx.push(Fx::Hit {
            kind,
            lane,
            at: (tx, ty),
            splash: k.splash.unwrap_or(0.0),
        });

        let mut struck = vec![(target, damage)];
        if let Some(radius) = k.splash.filter(|r| *r > 0.0) {
            struck.clear();
            for u in self.units.values() {
                if u.lane != lane || u.team == team || !u.alive() {
                    continue;
                }
                if !content.units[u.kind].moves_on.intersects(k.targets) {
                    continue;
                }
                let d = dist(u.x, u.y, tx, ty);
                if d <= radius {
                    let edge = k.splash_falloff / 100.0;
                    struck.push((u.id, damage * (1.0 - (1.0 - edge) * d / radius)));
                }
            }
        }
        for (u, dmg) in &struck {
            self.damage_unit(*u, *dmg, &k.damage_type, source);
            for e in &k.on_hit {
                self.apply_effect(*u, e, Some(source));
            }
        }

        for bounce in k.on_hit.iter().filter(|e| e.kind == EffectKind::Bounce) {
            let mut hit: Vec<UnitId> = vec![target];
            let (mut px, mut py) = (tx, ty);
            let mut dmg = damage;
            for _ in 0..bounce.count {
                let mut best: Option<(f64, UnitId, f64, f64)> = None;
                for u in self.units.values() {
                    if u.lane != lane || u.team == team || !u.alive() || hit.contains(&u.id) {
                        continue;
                    }
                    if !content.units[u.kind].moves_on.intersects(k.targets) {
                        continue;
                    }
                    let d = dist(u.x, u.y, px, py);
                    if d <= bounce.range && best.is_none_or(|(bd, ..)| d < bd) {
                        best = Some((d, u.id, u.x, u.y));
                    }
                }
                let Some((_, next, nx, ny)) = best else { break };
                dmg *= bounce.falloff / 100.0;
                self.fx.push(Fx::Bounce {
                    kind,
                    lane,
                    from: (px, py),
                    to: (nx, ny),
                });
                self.damage_unit(next, dmg, &k.damage_type, source);
                for e in k.on_hit.iter().filter(|e| e.kind != EffectKind::Bounce) {
                    self.apply_effect(next, e, Some(source));
                }
                hit.push(next);
                (px, py) = (nx, ny);
            }
        }
    }

    fn units_attack(&mut self) {
        let ids: Vec<UnitId> = self.units.keys().copied().collect();
        for id in ids {
            let u = self.units.get_mut(&id).unwrap();
            u.attack_cooldown = u.attack_cooldown.saturating_sub(1);
            if !u.alive() || u.attack_cooldown > 0 || u.effect(EffectKind::Stun).is_some() {
                continue;
            }
            let Some(sid) = u.attack_target else { continue };
            let Some(attack) = self.content.units[u.kind].attack.clone() else {
                continue;
            };
            let Some(s) = self.structures.get(&sid) else {
                continue;
            };
            let sk = &self.content.structures[s.kind];
            let armor = sk.armor + s.buff(EffectKind::Armor);
            let dealt = attack.damage
                * self
                    .content
                    .damage
                    .multiplier(&attack.damage_type, &sk.armor_type)
                * armor_factor(armor);
            let to = s.center(sk.size);
            let u = self.units.get_mut(&id).unwrap();
            u.attack_cooldown = secs_to_ticks(attack.cooldown, 1);
            let (from, lane) = ((u.x, u.y), u.lane);
            self.fx.push(Fx::UnitAttack {
                unit: id,
                lane,
                from,
                to,
            });
            let s = self.structures.get_mut(&sid).unwrap();
            if let Some(hp) = &mut s.hp {
                *hp -= dealt;
                if *hp <= 0.0 && s.killer.is_none() {
                    s.killer = Some(id);
                }
            }
        }
    }

    fn resolve(&mut self) {
        let gone: Vec<UnitId> = self
            .units
            .values()
            .filter(|u| !u.alive())
            .map(|u| u.id)
            .collect();
        for id in gone {
            let u = self.units.remove(&id).unwrap();
            let kind = self.content.units[u.kind].id.clone();
            if u.hp <= 0.0 {
                let killer_owner = match u.killer {
                    Some(Killer::Structure(s)) => self.structures.get(&s).map(|s| s.owner),
                    Some(Killer::Unit(k)) => self.units.get(&k).map(|k| k.owner),
                    None => None,
                };
                self.fx.push(Fx::UnitDied {
                    kind: u.kind,
                    lane: u.lane,
                    at: (u.x, u.y),
                });
                self.emit(Event::UnitDied {
                    unit: id,
                    kind,
                    owner: u.owner,
                    lane: u.lane,
                    killer: u.killer,
                    killer_owner,
                });
            } else {
                self.fx.push(Fx::UnitLeaked {
                    kind: u.kind,
                    lane: u.lane,
                    at: (u.x, u.y),
                });
                self.emit(Event::UnitReachedExit {
                    unit: id,
                    kind,
                    owner: u.owner,
                    lane: u.lane,
                });
            }
        }
        let destroyed: Vec<StructureId> = self
            .structures
            .values()
            .filter(|s| s.hp.is_some_and(|hp| hp <= 0.0))
            .map(|s| s.id)
            .collect();
        for id in destroyed {
            let s = self.remove_structure(id).unwrap();
            self.fx.push(Fx::StructureRemoved {
                kind: s.kind,
                lane: s.lane,
                at: (s.x, s.y),
                destroyed: true,
            });
            let kind = self.content.structures[s.kind].id.clone();
            self.emit(Event::StructureDestroyed {
                structure: id,
                kind,
                owner: s.owner,
                killer: s.killer,
            });
        }
    }
}

fn add_effect(effects: &mut Vec<ActiveEffect>, def: &EffectDef, source: Option<StructureId>) {
    let remaining = secs_to_ticks(def.duration, 1);
    match effects.iter_mut().find(|e| e.def.kind == def.kind) {
        Some(existing) => {
            if def.strength() >= existing.def.strength() {
                // Equal or stronger replaces and refreshes.
                *existing = ActiveEffect {
                    def: def.clone(),
                    remaining,
                    source,
                };
            }
        }
        None => effects.push(ActiveEffect {
            def: def.clone(),
            remaining,
            source,
        }),
    }
}

fn dist(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (ax - bx, ay - by);
    (dx * dx + dy * dy).sqrt()
}

/// Moves toward `(tx, ty)` by at most `budget`; returns the budget left.
fn step_toward(
    x: &mut f64,
    y: &mut f64,
    facing: &mut (f64, f64),
    tx: f64,
    ty: f64,
    budget: f64,
) -> f64 {
    let (dx, dy) = (tx - *x, ty - *y);
    let d = (dx * dx + dy * dy).sqrt();
    if d <= budget {
        *x = tx;
        *y = ty;
        if d > 0.0 {
            *facing = (dx / d, dy / d);
        }
        budget - d
    } else {
        *x += dx / d * budget;
        *y += dy / d * budget;
        *facing = (dx / d, dy / d);
        0.0
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}
