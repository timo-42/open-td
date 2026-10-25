//! `lives`: a leak subtracts from a resource; whoever still has some when
//! the waves end, or is last standing, wins.
//!
//! ```toml
//! [params.lives]
//! resource = "lives"   # any name
//! scope = "player"     # or "team": one shared pool per team
//! start = 20
//! cost = 1             # per leaked unit
//! costs = { boss = 10 } # per unit kind, overrides `cost`
//! ```
//!
//! Leaks on a shared lane cost every team. Listens for the `waves_done`
//! signal.

use std::collections::{BTreeMap, BTreeSet};

use td_mod_sdk::prelude::*;

#[derive(Default)]
struct Lives {
    resource: String,
    team_scope: bool,
    start: i64,
    cost: i64,
    costs: BTreeMap<String, i64>,
    players: Vec<PlayerInfo>,
    lanes: BTreeMap<LaneId, LaneOwner>,
    values: BTreeMap<Scope, i64>,
    out: BTreeSet<Scope>,
    ended: bool,
}

impl Lives {
    fn scope_of(&self, p: PlayerId) -> Scope {
        if self.team_scope {
            Scope::Team(self.players[p as usize].team)
        } else {
            Scope::Player(p)
        }
    }

    fn scopes(&self) -> Vec<Scope> {
        let mut s: Vec<Scope> = self.players.iter().map(|p| self.scope_of(p.id)).collect();
        s.dedup();
        s.sort();
        s.dedup();
        s
    }

    fn players_in(&self, scope: Scope) -> Vec<PlayerId> {
        self.players
            .iter()
            .filter(|p| self.scope_of(p.id) == scope)
            .map(|p| p.id)
            .collect()
    }

    fn charged_for(&self, lane: LaneId) -> Vec<Scope> {
        match self.lanes.get(&lane) {
            Some(LaneOwner::Player(p)) => vec![self.scope_of(*p)],
            Some(LaneOwner::Team(t)) => {
                if self.team_scope {
                    vec![Scope::Team(*t)]
                } else {
                    self.players
                        .iter()
                        .filter(|p| p.team == *t)
                        .map(|p| Scope::Player(p.id))
                        .collect()
                }
            }
            Some(LaneOwner::Shared) | None => self.scopes(),
        }
    }

    fn end(&mut self, winners: Vec<PlayerId>, text: String, out: &mut Vec<Command>) {
        if self.ended {
            return;
        }
        self.ended = true;
        let losers = self
            .players
            .iter()
            .map(|p| p.id)
            .filter(|p| !winners.contains(p))
            .collect();
        out.push(Command::EndGame {
            result: GameResult {
                winners,
                losers,
                text,
            },
        });
    }

    fn check_last_standing(&mut self, out: &mut Vec<Command>) {
        let scopes = self.scopes();
        let alive: Vec<Scope> = scopes
            .iter()
            .copied()
            .filter(|s| !self.out.contains(s))
            .collect();
        if alive.is_empty() {
            self.end(vec![], "Defeat".into(), out);
            return;
        }
        // Last standing only means something when there were rivals.
        let teams: BTreeSet<TeamId> = self.players.iter().map(|p| p.team).collect();
        let alive_teams: BTreeSet<TeamId> = alive
            .iter()
            .flat_map(|s| self.players_in(*s))
            .map(|p| self.players[p as usize].team)
            .collect();
        if teams.len() > 1 && alive_teams.len() == 1 {
            let winners = alive.iter().flat_map(|s| self.players_in(*s)).collect();
            self.end(winners, "Last one standing".into(), out);
        }
    }
}

impl Module for Lives {
    fn init(&mut self, init: ModuleInit) {
        let p = &init.params;
        self.resource = p.get("resource").str_or("lives").to_string();
        self.team_scope = p.get("scope").str_or("player") == "team";
        self.start = p.get("start").i64_or(20);
        self.cost = p.get("cost").i64_or(1);
        self.costs = p.get("costs").as_amounts();
        self.players = init.players;
    }

    fn on_event(&mut self, event: Event, out: &mut Vec<Command>) {
        if self.ended {
            return;
        }
        match event {
            Event::GameStart { lanes, players } => {
                self.players = players;
                self.lanes = lanes.iter().map(|l| (l.id, l.owner)).collect();
                for scope in self.scopes() {
                    out.push(Command::AddResource {
                        scope,
                        name: self.resource.clone(),
                        delta: self.start,
                        allow_negative: false,
                    });
                }
            }
            Event::UnitReachedExit {
                kind, lane, owner, ..
            } => {
                let cost = self.costs.get(&kind).copied().unwrap_or(self.cost);
                for scope in self.charged_for(lane) {
                    // A team never pays for its own sent units.
                    let own = self.players_in(scope).contains(&owner);
                    if !self.out.contains(&scope) && !own {
                        out.push(Command::AddResource {
                            scope,
                            name: self.resource.clone(),
                            delta: -cost,
                            allow_negative: true,
                        });
                    }
                }
            }
            Event::ResourceChanged {
                scope, name, value, ..
            } if name == self.resource => {
                self.values.insert(scope, value);
                if value <= 0 && !self.out.contains(&scope) {
                    self.out.insert(scope);
                    let who = match scope {
                        Scope::Player(p) => self.players[p as usize].name.clone(),
                        Scope::Team(t) => format!("Team {}", t + 1),
                    };
                    out.push(Command::Message {
                        to: MessageTarget::All,
                        text: format!("{who} is out of {}", self.resource),
                    });
                    self.check_last_standing(out);
                }
            }
            Event::PlayerLeft { player } => {
                let scope = self.scope_of(player);
                if !self.team_scope || self.players_in(scope).len() == 1 {
                    self.out.insert(scope);
                }
                self.check_last_standing(out);
            }
            Event::Signal { name, .. } if name == "waves_done" => {
                let winners: Vec<PlayerId> = self
                    .scopes()
                    .into_iter()
                    .filter(|s| !self.out.contains(s))
                    .flat_map(|s| self.players_in(s))
                    .collect();
                let text = if winners.is_empty() {
                    "Defeat".to_string()
                } else {
                    "Victory".to_string()
                };
                self.end(winners, text, out);
            }
            _ => {}
        }
    }
}

td_mod_sdk::export_module!(Lives);
