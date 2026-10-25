//! A simple greedy bot: builds a rotation of towers where they cover the
//! most path, upgrades when rich, and uses a builder when the rules demand
//! one. Knows nothing about resource names; it reads costs from data.

use td_core::lane::UNREACHABLE;
use td_core::sim::UnitMode;
use td_core::{Lane, Sim, TileFlags};
use td_mod_api::{Cell, Command, PlayerId, Scope};

pub struct Bot {
    pub player: PlayerId,
    /// Ticks between decisions.
    pub period: u64,
    /// Requests sent each decision when affordable, e.g. `send_monster`.
    pub requests: Vec<(String, td_mod_api::Value)>,
    turn: usize,
}

impl Bot {
    pub fn new(player: PlayerId) -> Bot {
        Bot {
            player,
            period: 15,
            requests: Vec::new(),
            turn: 0,
        }
    }

    /// Commands for this tick (usually none).
    pub fn think(&mut self, sim: &mut Sim) -> Vec<Command> {
        let mut out = Vec::new();
        if sim.tick % self.period != (self.player as u64 * 3) % self.period {
            return out;
        }
        let scope = Scope::Player(self.player);
        let content = sim.content.clone();
        let towers: Vec<usize> = (0..content.structures.len())
            .filter(|&k| content.structures[k].can_attack())
            .filter(|&k| {
                !content
                    .structures
                    .iter()
                    .any(|s| s.upgrades_to.contains(&content.structures[k].id))
            })
            .filter(|&k| sim.allowed(self.player).is_none_or(|a| a.contains(&k)))
            .collect();
        if towers.is_empty() {
            return out;
        }
        let mut sorted = towers.clone();
        sorted.sort_by_key(|&k| (content.structures[k].cost.values().sum::<i64>(), k));
        // Cheap towers first, then a rotation over everything buildable.
        let pool = &sorted[..sorted.len().min(2 + self.turn / 3)];
        let kind = pool[self.turn % pool.len()];

        let builder = if sim.requires_builder(self.player) {
            let b = sim
                .units
                .values()
                .find(|u| u.owner == self.player && matches!(u.mode, UnitMode::Idle))
                .map(|u| u.id);
            if b.is_none() {
                return out;
            }
            b
        } else {
            None
        };

        if sim.can_afford(scope, &content.structures[kind].cost)
            && let Some(at) = self.best_site(sim, kind)
        {
            out.push(Command::Build {
                kind: content.structures[kind].id.clone(),
                at,
                owner: self.player,
                builder,
            });
            self.turn += 1;
            return out;
        }

        // Rich enough after a while: upgrade the oldest tower that can be.
        let mine = sim
            .structures
            .values()
            .filter(|s| s.owner == self.player)
            .count();
        if mine >= 6 {
            for s in sim
                .structures
                .values()
                .filter(|s| s.owner == self.player && s.constructing == 0)
            {
                let k = &content.structures[s.kind];
                if let Some(next) = k.upgrades_to.first().and_then(|u| content.structure(u))
                    && sim.can_afford(scope, &content.structures[next].cost)
                {
                    out.push(Command::Upgrade {
                        structure: s.id,
                        to: content.structures[next].id.clone(),
                    });
                    return out;
                }
            }
        }

        for (request, data) in &self.requests {
            out.push(Command::Request {
                request: request.clone(),
                data: data.clone(),
            });
        }
        out
    }

    /// The buildable site whose range covers the most walkable path.
    fn best_site(&self, sim: &mut Sim, kind: usize) -> Option<Cell> {
        let k = sim.content.structures[kind].clone();
        let mut candidates = Vec::new();
        for lane in &sim.lanes {
            if !sim.can_build_on(self.player, lane.id) {
                continue;
            }
            let path: Vec<(f64, f64)> = path_tiles(lane, k.targets);
            for y in 0..lane.height as i32 {
                for x in 0..lane.width as i32 {
                    let ok = Lane::footprint(x, y, k.size).all(|(fx, fy)| {
                        lane.in_bounds(fx, fy) && {
                            let i = lane.idx(fx, fy);
                            lane.occupant[i].is_none() && lane.base[i].contains(TileFlags::BUILD)
                        }
                    });
                    if !ok {
                        continue;
                    }
                    let (cx, cy) = (
                        x as f64 + k.size as f64 / 2.0,
                        y as f64 + k.size as f64 / 2.0,
                    );
                    let r2 = k.range * k.range;
                    let score = path
                        .iter()
                        .filter(|(px, py)| (px - cx) * (px - cx) + (py - cy) * (py - cy) <= r2)
                        .count();
                    if score > 0 {
                        candidates.push((score, Cell::new(lane.id, x, y)));
                    }
                }
            }
        }
        // Highest score first; stable on position for determinism.
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .take(12)
            .map(|(_, c)| c)
            .find(|&c| sim.check_placement(kind, c, self.player).is_ok())
    }
}

/// Centres of tiles units actually walk (or fly) through, for masks the
/// structure can hit.
fn path_tiles(lane: &Lane, targets: TileFlags) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for f in lane
        .fields
        .iter()
        .filter(|f| f.guarded && f.mask.intersects(targets) && !f.mask.contains(TileFlags::UNIT))
    {
        // Walk the route from each spawn: tiles on the current path only.
        for &(sx, sy) in &lane.spawns {
            let (mut x, mut y) = (sx, sy);
            for _ in 0..lane.base.len() {
                out.push((x as f64 + 0.5, y as f64 + 0.5));
                if lane.is_exit(x, y) || f.dist[lane.idx(x, y)] == UNREACHABLE {
                    break;
                }
                match lane.next_step(&f.dist, f.mask, x, y) {
                    Some((nx, ny)) => (x, y) = (nx, ny),
                    None => break,
                }
            }
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap());
    out.dedup();
    out
}
