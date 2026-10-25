//! `versus`: a `send_monster` request spends resources and spawns the bought
//! unit into an opposing team's lane.
//!
//! ```toml
//! [params.versus]
//! # what may be sent, and its price
//! units = { grunt = { gold = 10 }, wolf = { gold = 25 } }
//! # optional per-wave income raise per send (signalled to `income`)
//! income = { grunt = { gold = 1 }, wolf = { gold = 3 } }
//! ```
//!
//! Request data: `{ unit = "grunt" }`. Targets rotate over the opposing
//! lanes. Sent units are owned by the sender.

use std::collections::BTreeMap;

use td_mod_sdk::prelude::*;

#[derive(Default)]
struct Versus {
    prices: BTreeMap<String, BTreeMap<String, i64>>,
    income: BTreeMap<String, BTreeMap<String, i64>>,
    players: Vec<PlayerInfo>,
    lanes: Vec<LaneInfo>,
    rotation: BTreeMap<PlayerId, usize>,
}

impl Versus {
    fn targets(&self, sender: PlayerId) -> Vec<LaneId> {
        let team = self.players[sender as usize].team;
        self.lanes
            .iter()
            .filter(|l| !l.spawns.is_empty())
            .filter(|l| match l.owner {
                LaneOwner::Player(p) => {
                    self.players.get(p as usize).is_some_and(|o| o.team != team)
                }
                LaneOwner::Team(t) => t != team,
                LaneOwner::Shared => false,
            })
            .map(|l| l.id)
            .collect()
    }
}

impl Module for Versus {
    fn init(&mut self, init: ModuleInit) {
        if let Some(units) = init.params.get("units").as_map() {
            for (id, cost) in units {
                self.prices.insert(id.clone(), cost.as_amounts());
            }
        }
        if let Some(units) = init.params.get("income").as_map() {
            for (id, amounts) in units {
                self.income.insert(id.clone(), amounts.as_amounts());
            }
        }
    }

    fn on_event(&mut self, event: Event, out: &mut Vec<Command>) {
        match event {
            Event::GameStart { players, lanes } => {
                self.players = players;
                self.lanes = lanes;
            }
            Event::PlayerRequest {
                player,
                request,
                data,
            } if request == "send_monster" => {
                let Some(unit) = data.get("unit").as_str() else {
                    return;
                };
                let Some(cost) = self.prices.get(unit) else {
                    out.push(Command::Message {
                        to: MessageTarget::Player(player),
                        text: format!("{unit} can't be sent"),
                    });
                    return;
                };
                let targets = self.targets(player);
                if targets.is_empty() {
                    out.push(Command::Message {
                        to: MessageTarget::Player(player),
                        text: "no opponent to send to".into(),
                    });
                    return;
                }
                let turn = self.rotation.entry(player).or_default();
                let lane = targets[*turn % targets.len()];
                *turn += 1;
                let mut then = vec![Command::Spawn {
                    kind: unit.into(),
                    lane,
                    owner: player,
                    hp_percent: 100,
                    at: None,
                    idle: false,
                }];
                if let Some(bonus) = self.income.get(unit) {
                    let mut data = Value::map();
                    data.insert("player", Value::Int(player as i64));
                    data.insert(
                        "amounts",
                        Value::Map(
                            bonus
                                .iter()
                                .map(|(k, v)| (k.clone(), Value::Int(*v)))
                                .collect(),
                        ),
                    );
                    then.push(Command::Signal {
                        name: "income_bonus".into(),
                        data,
                    });
                }
                out.push(Command::Purchase {
                    scope: Scope::Player(player),
                    cost: cost.clone(),
                    then,
                    notify: Some(player),
                });
            }
            _ => {}
        }
    }
}

td_mod_sdk::export_module!(Versus);
