//! The running game: simulation, bots for the other players, the fixed-tick
//! clock, and what the HUD needs from the event stream.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use td_content::{GameOptions, Library};
use td_core::{Fx, Game};
use td_headless::Bot;
use td_mod_api::{Cell, Event, Map, MessageTarget, PlayerId, TICK_RATE, UnitId, Value};

use crate::Screen;

/// Gap between lanes, in tiles.
const LANE_GAP: f32 = 3.0;

/// The simulation step; drawing runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Advance;

pub fn plugin(app: &mut App) {
    app.add_systems(Update, start_pending.run_if(in_state(Screen::Menu)))
        .add_systems(
            Update,
            advance.in_set(Advance).run_if(in_state(Screen::Playing)),
        )
        .add_systems(OnExit(Screen::Playing), |mut commands: Commands| {
            commands.remove_resource::<Session>()
        });
}

/// A game to start; picked up in the menu state, which then switches to
/// playing. Used by the menu, "play again" and `--map`.
#[derive(Resource, Clone)]
pub struct PendingStart {
    pub map: Map,
    pub players: Option<u8>,
    pub seed: u64,
}

/// Why the last start failed, for the menu.
#[derive(Resource, Default)]
pub struct StartError(pub Option<String>);

fn start_pending(
    mut commands: Commands,
    pending: Option<Res<PendingStart>>,
    packs: Option<Res<crate::assets::Packs>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let (Some(p), Some(packs)) = (pending, packs) else {
        return;
    };
    commands.remove_resource::<PendingStart>();
    match Session::start(&packs.lib, &p.map, p.players, p.seed) {
        Ok(s) => {
            commands.insert_resource(s);
            commands.insert_resource(StartError(None));
            next.set(Screen::Playing);
        }
        Err(e) => {
            error!("{e}");
            commands.insert_resource(StartError(Some(e)));
        }
    }
}

pub struct LogLine {
    pub text: String,
    pub at: f32,
}

#[derive(Resource)]
pub struct Session {
    pub game: Game,
    pub map: Map,
    pub me: PlayerId,
    pub bots: Vec<Bot>,
    pub speed: f32,
    pub paused: bool,
    acc: f32,
    /// How far between the last tick and the next, for interpolation.
    pub alpha: f32,
    pub prev_units: HashMap<UnitId, Vec2>,
    pub prev_projectiles: HashMap<u32, Vec2>,
    /// World position of each lane's top-left corner.
    pub layout: Vec<Vec2>,
    pub log: VecDeque<LogLine>,
    pub wave: Option<(i64, i64)>,
    pub fx: Vec<Fx>,
    pub clock: f32,
    pub seed: u64,
    /// The map's `[params]` over the pack defaults.
    pub params: Value,
}

impl Session {
    pub fn start(
        lib: &Library,
        map: &Map,
        players: Option<u8>,
        seed: u64,
    ) -> Result<Session, String> {
        let options = GameOptions { players, seed };
        let params = td_content::resolve(lib, map, &options)?.params;
        let mut game = td_content::start(lib, map, &options)?;
        game.observe_events = true;
        let me = 0;
        let bots = game
            .sim
            .players
            .iter()
            .filter(|p| p.id != me)
            .map(|p| Bot::new(p.id))
            .collect();

        let n = game.sim.lanes.len();
        let cols = (n as f32).sqrt().ceil().max(1.0) as usize;
        let w = game.sim.lanes.iter().map(|l| l.width).max().unwrap_or(1) as f32;
        let h = game.sim.lanes.iter().map(|l| l.height).max().unwrap_or(1) as f32;
        let layout = (0..n)
            .map(|i| {
                Vec2::new(
                    (i % cols) as f32 * (w + LANE_GAP),
                    -((i / cols) as f32) * (h + LANE_GAP),
                )
            })
            .collect();
        Ok(Session {
            game,
            map: map.clone(),
            me,
            bots,
            speed: 1.0,
            paused: false,
            acc: 0.0,
            alpha: 0.0,
            prev_units: HashMap::new(),
            prev_projectiles: HashMap::new(),
            layout,
            log: VecDeque::new(),
            wave: None,
            fx: Vec::new(),
            clock: 0.0,
            seed,
            params,
        })
    }

    /// Centre of a lane position (tile units, y down) in world space.
    pub fn to_world(&self, lane: u16, x: f64, y: f64) -> Vec2 {
        let o = self.layout.get(lane as usize).copied().unwrap_or_default();
        Vec2::new(o.x + x as f32, o.y - y as f32)
    }

    pub fn tile_center(&self, lane: u16, x: i32, y: i32) -> Vec2 {
        self.to_world(lane, x as f64 + 0.5, y as f64 + 0.5)
    }

    pub fn to_cell(&self, p: Vec2) -> Option<Cell> {
        for (i, lane) in self.game.sim.lanes.iter().enumerate() {
            let o = self.layout[i];
            let (x, y) = ((p.x - o.x).floor() as i32, (o.y - p.y).floor() as i32);
            if lane.in_bounds(x, y) {
                return Some(Cell::new(i as u16, x, y));
            }
        }
        None
    }

    /// The world rectangle covering every lane: (min, max).
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for (i, l) in self.game.sim.lanes.iter().enumerate() {
            let o = self.layout[i];
            min = min.min(Vec2::new(o.x, o.y - l.height as f32));
            max = max.max(Vec2::new(o.x + l.width as f32, o.y));
        }
        (min, max)
    }

    pub fn my_lane(&self) -> Option<u16> {
        self.game
            .sim
            .lanes
            .iter()
            .find(|l| self.game.sim.can_build_on(self.me, l.id))
            .map(|l| l.id)
    }

    pub fn has_module(&self, name: &str) -> bool {
        self.game.rules().any(|(n, _)| n == name)
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.log.push_back(LogLine {
            text: text.into(),
            at: self.clock,
        });
        while self.log.len() > 8 {
            self.log.pop_front();
        }
    }

    pub fn queue(&mut self, command: td_mod_api::Command) {
        let me = self.me;
        self.game.queue_player(me, command);
    }

    fn tick(&mut self) {
        self.prev_units = self
            .game
            .sim
            .units
            .values()
            .map(|u| (u.id, Vec2::new(u.x as f32, u.y as f32)))
            .collect();
        self.prev_projectiles = self
            .game
            .sim
            .projectiles
            .iter()
            .map(|p| (p.id, Vec2::new(p.x as f32, p.y as f32)))
            .collect();
        for bot in &mut self.bots {
            for c in bot.think(&mut self.game.sim) {
                self.game.queue_player(bot.player, c);
            }
        }
        self.game.step();

        let my_team = self.game.sim.team_of(self.me);
        let messages: Vec<_> = std::mem::take(&mut self.game.sim.messages);
        for (to, text) in messages {
            let mine = match to {
                MessageTarget::All => true,
                MessageTarget::Player(p) => p == self.me,
                MessageTarget::Team(t) => t == my_team,
            };
            if mine {
                self.say(text);
            }
        }
        for e in self.game.take_events() {
            if let Event::Signal { name, data } = e
                && name == "wave"
            {
                self.wave = Some((data.get("n").i64_or(0), data.get("total").i64_or(0)));
            }
        }
        for line in self.game.sim.log.drain(..) {
            warn!("{line}");
        }
        let fx: Vec<Fx> = std::mem::take(&mut self.game.sim.fx);
        self.fx.extend(fx);
    }
}

fn advance(time: Res<Time>, mut s: ResMut<Session>) {
    let dt_real = time.delta_secs();
    s.clock += dt_real;
    if s.paused || s.game.is_over() {
        s.alpha = 1.0;
        return;
    }
    let dt = 1.0 / TICK_RATE as f32;
    s.acc += dt_real * s.speed;
    let mut steps = 0;
    while s.acc >= dt {
        s.tick();
        s.acc -= dt;
        steps += 1;
        if steps >= 16 {
            s.acc = 0.0;
            break;
        }
    }
    s.alpha = (s.acc / dt).clamp(0.0, 1.0);
}
