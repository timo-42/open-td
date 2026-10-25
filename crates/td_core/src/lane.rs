//! Lane grids and pathing. One algorithm: a distance field per movement
//! mask, computed from the exits over tiles matching the mask.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use td_mod_api::{LaneId, LaneOwner, StructureId};

use crate::flags::TileFlags;

pub const UNREACHABLE: u32 = u32::MAX;
const ORTHO: u32 = 10;
const DIAG: u32 = 14;

/// Neighbour offsets; the order is part of determinism.
const DIRS: [(i32, i32, u32); 8] = [
    (1, 0, ORTHO),
    (0, 1, ORTHO),
    (-1, 0, ORTHO),
    (0, -1, ORTHO),
    (1, 1, DIAG),
    (-1, 1, DIAG),
    (-1, -1, DIAG),
    (1, -1, DIAG),
];

/// What a lane is built from.
#[derive(Debug, Clone)]
pub struct LaneSpec {
    pub owner: LaneOwner,
    pub width: u32,
    pub height: u32,
    /// Tile kind index per cell, row-major.
    pub tiles: Vec<usize>,
    pub spawns: Vec<(i32, i32)>,
    pub exits: Vec<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct FlowField {
    pub mask: TileFlags,
    pub dist: Vec<u32>,
    /// Every spawn had a route when the game started, so placements must
    /// keep one.
    pub guarded: bool,
}

#[derive(Debug, Clone)]
pub struct Lane {
    pub id: LaneId,
    pub owner: LaneOwner,
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<usize>,
    pub base: Vec<TileFlags>,
    /// Flags currently removed by structures.
    pub blocked: Vec<TileFlags>,
    pub occupant: Vec<Option<StructureId>>,
    pub spawns: Vec<(i32, i32)>,
    pub exits: Vec<(i32, i32)>,
    pub fields: Vec<FlowField>,
}

impl Lane {
    pub fn new(id: LaneId, spec: LaneSpec, tile_flags: &[TileFlags], masks: &[TileFlags]) -> Lane {
        let n = (spec.width * spec.height) as usize;
        let base = spec.tiles.iter().map(|&t| tile_flags[t]).collect();
        let mut lane = Lane {
            id,
            owner: spec.owner,
            width: spec.width,
            height: spec.height,
            tiles: spec.tiles,
            base,
            blocked: vec![TileFlags::NONE; n],
            occupant: vec![None; n],
            spawns: spec.spawns,
            exits: spec.exits,
            fields: Vec::new(),
        };
        for &mask in masks {
            let dist = lane.compute_field(mask);
            let guarded = !lane.spawns.is_empty()
                && lane
                    .spawns
                    .iter()
                    .all(|&(x, y)| dist[lane.idx(x, y)] != UNREACHABLE);
            lane.fields.push(FlowField {
                mask,
                dist,
                guarded,
            });
        }
        lane
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    pub fn idx(&self, x: i32, y: i32) -> usize {
        y as usize * self.width as usize + x as usize
    }

    pub fn flags(&self, x: i32, y: i32) -> TileFlags {
        if !self.in_bounds(x, y) {
            return TileFlags::NONE;
        }
        let i = self.idx(x, y);
        self.base[i] & !self.blocked[i]
    }

    pub fn passable(&self, x: i32, y: i32, mask: TileFlags) -> bool {
        self.flags(x, y).intersects(mask)
    }

    pub fn is_exit(&self, x: i32, y: i32) -> bool {
        self.exits.contains(&(x, y))
    }

    pub fn field(&self, mask: TileFlags) -> Option<&FlowField> {
        self.fields.iter().find(|f| f.mask == mask)
    }

    pub fn dist(&self, mask: TileFlags, x: i32, y: i32) -> u32 {
        match self.field(mask) {
            Some(f) if self.in_bounds(x, y) => f.dist[self.idx(x, y)],
            _ => UNREACHABLE,
        }
    }

    /// Dijkstra from the exits. Diagonal steps need both orthogonal
    /// neighbours open, so units never clip a corner.
    pub fn compute_field(&self, mask: TileFlags) -> Vec<u32> {
        let goals: Vec<(i32, i32)> = self
            .exits
            .iter()
            .copied()
            .filter(|&(x, y)| self.passable(x, y, mask))
            .collect();
        self.dijkstra(&goals, mask)
    }

    fn dijkstra(&self, goals: &[(i32, i32)], mask: TileFlags) -> Vec<u32> {
        let mut dist = vec![UNREACHABLE; self.base.len()];
        let mut heap = BinaryHeap::new();
        for &(x, y) in goals {
            let i = self.idx(x, y);
            dist[i] = 0;
            heap.push(Reverse((0u32, i)));
        }
        while let Some(Reverse((d, i))) = heap.pop() {
            if d > dist[i] {
                continue;
            }
            let x = (i % self.width as usize) as i32;
            let y = (i / self.width as usize) as i32;
            for (dx, dy, cost) in DIRS {
                let (nx, ny) = (x + dx, y + dy);
                if !self.passable(nx, ny, mask) {
                    continue;
                }
                if dx != 0
                    && dy != 0
                    && !(self.passable(x + dx, y, mask) && self.passable(x, y + dy, mask))
                {
                    continue;
                }
                let ni = self.idx(nx, ny);
                let nd = d + cost;
                if nd < dist[ni] {
                    dist[ni] = nd;
                    heap.push(Reverse((nd, ni)));
                }
            }
        }
        dist
    }

    /// The neighbour to step to from `(x, y)` along a distance field.
    pub fn next_step(&self, dist: &[u32], mask: TileFlags, x: i32, y: i32) -> Option<(i32, i32)> {
        let here = if self.in_bounds(x, y) {
            dist[self.idx(x, y)]
        } else {
            UNREACHABLE
        };
        let mut best: Option<((i32, i32), u32)> = None;
        for (dx, dy, _) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            if !self.passable(nx, ny, mask) {
                continue;
            }
            if dx != 0
                && dy != 0
                && !(self.passable(x + dx, y, mask) && self.passable(x, y + dy, mask))
            {
                continue;
            }
            let d = dist[self.idx(nx, ny)];
            if d == UNREACHABLE {
                continue;
            }
            // Standing on an unreachable tile (e.g. a flyer over a tile that
            // just lost its flag), step to any reachable neighbour.
            if here != UNREACHABLE && d >= here {
                continue;
            }
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some(((nx, ny), d));
            }
        }
        best.map(|(c, _)| c)
    }

    /// A tile path from `from` to `to` (inclusive of `to`), or `None`.
    pub fn path_to(
        &self,
        mask: TileFlags,
        from: (i32, i32),
        to: (i32, i32),
    ) -> Option<Vec<(i32, i32)>> {
        if !self.passable(to.0, to.1, mask) {
            return None;
        }
        let dist = self.dijkstra(&[to], mask);
        let mut path = Vec::new();
        let mut cur = from;
        let mut guard = self.base.len() + 1;
        while cur != to {
            cur = self.next_step(&dist, mask, cur.0, cur.1)?;
            path.push(cur);
            guard -= 1;
            if guard == 0 {
                return None;
            }
        }
        Some(path)
    }

    /// Cells of a footprint with its top-left corner at `(x, y)`.
    pub fn footprint(x: i32, y: i32, size: u8) -> impl Iterator<Item = (i32, i32)> {
        let s = size as i32;
        (0..s).flat_map(move |dy| (0..s).map(move |dx| (x + dx, y + dy)))
    }

    /// Recomputes the fields affected by a change to `mask` flags.
    pub fn recompute(&mut self, changed: TileFlags) {
        for i in 0..self.fields.len() {
            if self.fields[i].mask.intersects(changed) {
                let dist = self.compute_field(self.fields[i].mask);
                self.fields[i].dist = dist;
            }
        }
    }
}
