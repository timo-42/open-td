//! A game: the simulation plus the rule modules that drive it.

use td_mod_api::{Command, Event, MessageTarget, PlayerId};

use crate::sim::{Audience, Sim, Source};

/// A rule module. `td_mods` implements this for wasm; tests implement it in
/// Rust directly.
pub trait Rule: Send + Sync {
    fn name(&self) -> &str;
    /// React to an event; push commands for the next tick. An error disables
    /// the rule for the rest of the game.
    fn on_event(&mut self, event: &Event, out: &mut Vec<Command>) -> Result<(), String>;
}

struct Slot {
    rule: Box<dyn Rule>,
    enabled: bool,
}

pub struct Game {
    pub sim: Sim,
    rules: Vec<Slot>,
    /// When set, every event is also kept for the host (clients show wave
    /// signals and resource changes from it). Drain with [`Game::take_events`].
    pub observe_events: bool,
    observed: Vec<Event>,
}

impl Game {
    pub fn new(sim: Sim, rules: Vec<Box<dyn Rule>>) -> Game {
        Game {
            sim,
            rules: rules
                .into_iter()
                .map(|rule| Slot {
                    rule,
                    enabled: true,
                })
                .collect(),
            observe_events: false,
            observed: Vec::new(),
        }
    }

    /// Queue a player command for the next tick.
    pub fn queue_player(&mut self, player: PlayerId, command: Command) {
        self.sim.queue(Source::Player(player), command);
    }

    pub fn is_over(&self) -> bool {
        self.sim.result().is_some()
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.observed)
    }

    /// Names and enabled state of the rules, in run order.
    pub fn rules(&self) -> impl Iterator<Item = (&str, bool)> {
        self.rules.iter().map(|s| (s.rule.name(), s.enabled))
    }

    /// Advance one tick. Modules run in list order; each sees every event
    /// meant for it, and its commands are applied at the start of the next
    /// tick.
    pub fn step(&mut self) {
        if self.is_over() {
            return;
        }
        let events = self.sim.step();
        let mut out = Vec::new();
        for i in 0..self.rules.len() {
            if !self.rules[i].enabled {
                continue;
            }
            let subscribed = self.sim.is_tick_subscriber(i);
            for routed in &events {
                let wanted = match routed.audience {
                    Audience::All => true,
                    Audience::Module(m) => m == i,
                    Audience::TickSubscribers => subscribed,
                };
                if !wanted {
                    continue;
                }
                let slot = &mut self.rules[i];
                if let Err(e) = slot.rule.on_event(&routed.event, &mut out) {
                    slot.enabled = false;
                    out.clear();
                    let text = format!("rule module `{}` stopped: {e}", slot.rule.name());
                    self.sim.log.push(text.clone());
                    self.sim.messages.push((MessageTarget::All, text));
                    break;
                }
            }
            for c in out.drain(..) {
                self.sim.queue(Source::Module(i), c);
            }
        }
        if self.observe_events {
            self.observed.extend(events.into_iter().map(|r| r.event));
        }
    }

    /// Run until the game ends or `max_ticks` pass.
    pub fn run(&mut self, max_ticks: u64) {
        let end = self.sim.tick + max_ticks;
        while !self.is_over() && self.sim.tick < end {
            self.step();
        }
    }
}
