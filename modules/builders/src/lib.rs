//! `builders`: each player starts with one builder unit; `buy_builder`
//! spends resources for another. With this module, `Build` must name a
//! builder, who walks to the site first.
//!
//! ```toml
//! [params.builders]
//! unit = "peasant"            # builder unit id
//! start = 1
//! max = 2
//! cost = { gold = 50 }
//! palette = ["arrow_tower"]   # optional: restrict what can be built
//! ```

use std::collections::BTreeMap;

use td_mod_sdk::prelude::*;

#[derive(Default)]
struct Builders {
    unit: String,
    start: u32,
    max: u32,
    cost: BTreeMap<String, i64>,
    palette: Option<Vec<String>>,
    home: BTreeMap<PlayerId, LaneInfo>,
    count: BTreeMap<PlayerId, u32>,
}

impl Builders {
    fn spawn(&self, player: PlayerId) -> Option<Command> {
        let lane = self.home.get(&player)?;
        Some(Command::Spawn {
            kind: self.unit.clone(),
            lane: lane.id,
            owner: player,
            hp_percent: 100,
            at: lane.spawns.first().copied(),
            idle: true,
        })
    }
}

impl Module for Builders {
    fn init(&mut self, init: ModuleInit) {
        let p = &init.params;
        self.unit = p.get("unit").str_or("peasant").to_string();
        self.start = p.get("start").i64_or(1).max(0) as u32;
        self.max = p.get("max").i64_or(2).max(1) as u32;
        self.cost = p.get("cost").as_amounts();
        if !p.get("palette").is_nil() {
            self.palette = Some(
                p.get("palette")
                    .as_array()
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
            );
        }
    }

    fn on_event(&mut self, event: Event, out: &mut Vec<Command>) {
        match event {
            Event::GameStart { players, lanes } => {
                for p in &players {
                    let home = lanes
                        .iter()
                        .find(|l| l.owner == LaneOwner::Player(p.id))
                        .or_else(|| lanes.iter().find(|l| l.owner == LaneOwner::Team(p.team)))
                        .or_else(|| lanes.iter().find(|l| l.owner == LaneOwner::Shared));
                    if let Some(lane) = home {
                        self.home.insert(p.id, lane.clone());
                    }
                    out.push(Command::SetRequireBuilder {
                        player: p.id,
                        required: true,
                    });
                    if let Some(palette) = &self.palette {
                        out.push(Command::SetAllowed {
                            player: p.id,
                            structures: Some(palette.clone()),
                        });
                    }
                    for _ in 0..self.start {
                        out.extend(self.spawn(p.id));
                    }
                }
            }
            Event::UnitSpawned { kind, owner, .. } if kind == self.unit => {
                *self.count.entry(owner).or_default() += 1;
            }
            Event::UnitDied { kind, owner, .. } if kind == self.unit => {
                let c = self.count.entry(owner).or_default();
                *c = c.saturating_sub(1);
            }
            Event::PlayerRequest {
                player, request, ..
            } if request == "buy_builder" => {
                if self.count.get(&player).copied().unwrap_or(0) >= self.max {
                    out.push(Command::Message {
                        to: MessageTarget::Player(player),
                        text: format!("at most {} builders", self.max),
                    });
                    return;
                }
                if let Some(spawn) = self.spawn(player) {
                    out.push(Command::Purchase {
                        scope: Scope::Player(player),
                        cost: self.cost.clone(),
                        then: vec![spawn],
                        notify: Some(player),
                    });
                }
            }
            _ => {}
        }
    }
}

td_mod_sdk::export_module!(Builders);
