//! `waves`: reads the map's `[[wave]]` table and spawns creeps on timers.
//!
//! ```toml
//! [[wave]]
//! unit = "grunt"      # unit id
//! count = 10
//! interval = 0.5      # seconds between spawns
//! hp_percent = 100    # optional
//! delay = 20          # optional: seconds before this wave, after the last
//!
//! [params.waves]
//! first_delay = 15    # seconds before wave 1
//! delay = 25          # default seconds between waves
//! owner = 255         # who owns the creeps; neutral by default
//! ```
//!
//! A wave entry may also hold `groups = [{ unit, count, interval }, …]` to
//! mix units. Signals `wave` `{ n, total }` when a wave starts and
//! `waves_done` once every wave has spawned and every creep it spawned is
//! gone. Players may request `next_wave` to call the next wave early.

use td_mod_sdk::prelude::*;

const TIMER_NEXT_WAVE: u32 = 1;
const TIMER_SPAWN: u32 = 2;

#[derive(Clone, Debug)]
struct Group {
    unit: String,
    count: u32,
    interval: u32,
}

#[derive(Clone, Debug)]
struct Wave {
    groups: Vec<Group>,
    hp_percent: u32,
    delay: Option<u32>,
    name: Option<String>,
}

#[derive(Default)]
struct Waves {
    waves: Vec<Wave>,
    first_delay: u32,
    delay: u32,
    owner: PlayerId,
    lanes: Vec<LaneId>,
    current: usize,
    /// (group, spawned in group) while a wave is spawning.
    spawning: Option<(usize, u32)>,
    alive: u32,
    done: bool,
}

fn ticks(secs: f64) -> u32 {
    ((secs * TICK_RATE as f64) + 0.5).max(1.0) as u32
}

fn group(v: &Value) -> Option<Group> {
    Some(Group {
        unit: v.get("unit").as_str()?.to_string(),
        count: v.get("count").i64_or(1).max(0) as u32,
        interval: ticks(v.get("interval").f64_or(1.0)),
    })
}

impl Waves {
    fn start_wave(&mut self, out: &mut Vec<Command>) {
        let n = self.current + 1;
        let wave = &self.waves[self.current];
        let what: Vec<String> = wave
            .groups
            .iter()
            .map(|g| format!("{} x{}", g.unit, g.count))
            .collect();
        let title = wave.name.clone().unwrap_or_else(|| format!("Wave {n}"));
        out.push(Command::Message {
            to: MessageTarget::All,
            text: format!("{title}: {}", what.join(", ")),
        });
        let mut data = Value::map();
        data.insert("n", Value::Int(n as i64));
        data.insert("total", Value::Int(self.waves.len() as i64));
        out.push(Command::Signal {
            name: "wave".into(),
            data,
        });
        self.spawning = Some((0, 0));
        self.spawn_next(out);
    }

    fn spawn_next(&mut self, out: &mut Vec<Command>) {
        let Some((gi, spawned)) = self.spawning else {
            return;
        };
        let wave = &self.waves[self.current];
        let Some(g) = wave.groups.get(gi) else {
            self.finish_spawning(out);
            return;
        };
        if spawned >= g.count {
            self.spawning = Some((gi + 1, 0));
            return self.spawn_next(out);
        }
        for &lane in &self.lanes {
            out.push(Command::Spawn {
                kind: g.unit.clone(),
                lane,
                owner: self.owner,
                hp_percent: wave.hp_percent,
                at: None,
                idle: false,
            });
        }
        self.spawning = Some((gi, spawned + 1));
        out.push(Command::SetTimer {
            id: TIMER_SPAWN,
            ticks: g.interval,
            repeat: false,
        });
    }

    fn finish_spawning(&mut self, out: &mut Vec<Command>) {
        self.spawning = None;
        self.current += 1;
        if self.current < self.waves.len() {
            let delay = self.waves[self.current].delay.unwrap_or(self.delay);
            out.push(Command::SetTimer {
                id: TIMER_NEXT_WAVE,
                ticks: delay,
                repeat: false,
            });
        } else {
            self.check_done(out);
        }
    }

    fn check_done(&mut self, out: &mut Vec<Command>) {
        if !self.done
            && self.current >= self.waves.len()
            && self.spawning.is_none()
            && self.alive == 0
        {
            self.done = true;
            out.push(Command::Message {
                to: MessageTarget::All,
                text: "All waves cleared".into(),
            });
            out.push(Command::Signal {
                name: "waves_done".into(),
                data: Value::Nil,
            });
        }
    }
}

impl Module for Waves {
    fn init(&mut self, init: ModuleInit) {
        self.first_delay = ticks(init.params.get("first_delay").f64_or(15.0));
        self.delay = ticks(init.params.get("delay").f64_or(25.0));
        self.owner = init.params.get("owner").i64_or(NEUTRAL as i64) as PlayerId;
        for w in init.map.get("wave").as_array() {
            let mut groups: Vec<Group> = w
                .get("groups")
                .as_array()
                .iter()
                .filter_map(group)
                .collect();
            if let Some(g) = group(w) {
                groups.insert(0, g);
            }
            if groups.is_empty() {
                log("waves: skipping a wave without units");
                continue;
            }
            self.waves.push(Wave {
                groups,
                hp_percent: w.get("hp_percent").i64_or(100).max(1) as u32,
                delay: w.get("delay").as_f64().map(ticks),
                name: w.get("name").as_str().map(String::from),
            });
        }
    }

    fn on_event(&mut self, event: Event, out: &mut Vec<Command>) {
        match event {
            Event::GameStart { lanes, .. } => {
                self.lanes = lanes
                    .iter()
                    .filter(|l| !l.spawns.is_empty())
                    .map(|l| l.id)
                    .collect();
                if self.waves.is_empty() {
                    self.check_done(out);
                } else {
                    let secs = self.first_delay / TICK_RATE;
                    out.push(Command::Message {
                        to: MessageTarget::All,
                        text: format!("First wave in {secs} seconds"),
                    });
                    out.push(Command::SetTimer {
                        id: TIMER_NEXT_WAVE,
                        ticks: self.first_delay,
                        repeat: false,
                    });
                }
            }
            Event::Timer {
                id: TIMER_NEXT_WAVE,
            } if self.spawning.is_none() && self.current < self.waves.len() => self.start_wave(out),
            Event::Timer { id: TIMER_SPAWN } => self.spawn_next(out),
            Event::PlayerRequest { request, .. } if request == "next_wave" => {
                if self.spawning.is_none() && self.current < self.waves.len() {
                    out.push(Command::CancelTimer {
                        id: TIMER_NEXT_WAVE,
                    });
                    self.start_wave(out);
                }
            }
            Event::UnitSpawned { owner, .. } if owner == self.owner => self.alive += 1,
            Event::UnitDied { owner, .. } | Event::UnitReachedExit { owner, .. }
                if owner == self.owner =>
            {
                self.alive = self.alive.saturating_sub(1);
                self.check_done(out);
            }
            _ => {}
        }
    }
}

td_mod_sdk::export_module!(Waves);
