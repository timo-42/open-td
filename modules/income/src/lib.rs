//! `income`: starting resources, periodic income and kill bounty, for any
//! set of resource names.
//!
//! ```toml
//! [params.income]
//! start = { gold = 100, mana = 0 }   # each player, at game start
//! per_wave = { gold = 20 }           # each player, on every `wave` signal
//! interval = 0                       # seconds between periodic payments, 0 = off
//! per_interval = { gold = 5 }
//! bounty = true                      # pay a unit's `bounty` to its killer's owner
//! bounty_percent = 100
//! ```
//!
//! Other modules can raise a player's per-wave income with the
//! `income_bonus` signal: `{ player = 0, amounts = { gold = 2 } }`.

use std::collections::BTreeMap;

use td_mod_sdk::prelude::*;

const TIMER_INCOME: u32 = 1;

#[derive(Default)]
struct Income {
    start: BTreeMap<String, i64>,
    per_wave: BTreeMap<String, i64>,
    interval: u32,
    per_interval: BTreeMap<String, i64>,
    bounty: bool,
    bounty_percent: i64,
    bounties: BTreeMap<String, BTreeMap<String, i64>>,
    bonus: BTreeMap<PlayerId, BTreeMap<String, i64>>,
    players: Vec<PlayerId>,
}

fn pay(out: &mut Vec<Command>, player: PlayerId, amounts: &BTreeMap<String, i64>) {
    for (name, delta) in amounts {
        if *delta != 0 {
            out.push(Command::AddResource {
                scope: Scope::Player(player),
                name: name.clone(),
                delta: *delta,
                allow_negative: true,
            });
        }
    }
}

impl Module for Income {
    fn init(&mut self, init: ModuleInit) {
        let p = &init.params;
        self.start = p.get("start").as_amounts();
        self.per_wave = p.get("per_wave").as_amounts();
        self.interval = (p.get("interval").f64_or(0.0) * TICK_RATE as f64) as u32;
        self.per_interval = p.get("per_interval").as_amounts();
        self.bounty = p.get("bounty").as_bool().unwrap_or(true);
        self.bounty_percent = p.get("bounty_percent").i64_or(100);
        for u in init.data.get("unit").as_array() {
            if let Some(id) = u.get("id").as_str() {
                self.bounties
                    .insert(id.to_string(), u.get("bounty").as_amounts());
            }
        }
    }

    fn on_event(&mut self, event: Event, out: &mut Vec<Command>) {
        match event {
            Event::GameStart { players, .. } => {
                self.players = players.iter().map(|p| p.id).collect();
                for &p in &self.players {
                    pay(out, p, &self.start);
                }
                if self.interval > 0 {
                    out.push(Command::SetTimer {
                        id: TIMER_INCOME,
                        ticks: self.interval,
                        repeat: true,
                    });
                }
            }
            Event::Timer { id: TIMER_INCOME } => {
                for &p in &self.players {
                    pay(out, p, &self.per_interval);
                }
            }
            Event::Signal { name, data } if name == "wave" => {
                // Wave 1 is the start of play, not an income tick.
                if data.get("n").i64_or(1) > 1 {
                    for &p in &self.players {
                        let mut amounts = self.per_wave.clone();
                        for (k, v) in self.bonus.get(&p).into_iter().flatten() {
                            *amounts.entry(k.clone()).or_default() += v;
                        }
                        pay(out, p, &amounts);
                    }
                }
            }
            Event::Signal { name, data } if name == "income_bonus" => {
                let p = data.get("player").i64_or(-1);
                if p >= 0 {
                    let entry = self.bonus.entry(p as PlayerId).or_default();
                    for (k, v) in data.get("amounts").as_amounts() {
                        *entry.entry(k).or_default() += v;
                    }
                }
            }
            Event::UnitDied {
                kind,
                owner,
                killer_owner: Some(killer),
                ..
            } if self.bounty && killer != owner => {
                if let Some(b) = self.bounties.get(&kind) {
                    let scaled = b
                        .iter()
                        .map(|(k, v)| (k.clone(), v * self.bounty_percent / 100))
                        .collect();
                    pay(out, killer, &scaled);
                }
            }
            _ => {}
        }
    }
}

td_mod_sdk::export_module!(Income);
