//! `canyon_gen`: generates a winding canyon (fixed path) or an open field
//! (mazing) lane, one copy per player, with a scaled wave table.
//!
//! Params (declared in the base `info.toml`): `width`, `height`, `lanes`
//! (players), `style` (`canyon` or `maze`), `waves`, `seed`.

use std::collections::BTreeMap;

use td_mod_sdk::prelude::*;

const ROSTER: [&str; 8] = [
    "grunt", "runner", "wolf", "grunt", "bat", "knight", "wolf", "golem",
];
const BOSSES: [&str; 2] = ["ogre", "dragon"];

fn rand(n: i64) -> i64 {
    if n <= 0 {
        0
    } else {
        random_below(n as u64) as i64
    }
}

fn canyon(w: usize, h: usize) -> Vec<Vec<char>> {
    let mut g = vec![vec!['#'; w]; h];
    let mut y = 1 + rand(h as i64 - 2) as usize;
    let start = y;
    let mut last_turn = 0usize;
    for x in 0..w {
        g[y][x] = '.';
        if x >= 2 && x + 2 < w && x - last_turn >= 3 && rand(100) < 55 {
            let target = 1 + rand(h as i64 - 2) as usize;
            if target.abs_diff(y) >= 2 {
                let (a, b) = if target < y { (target, y) } else { (y, target) };
                for row in g.iter_mut().take(b + 1).skip(a) {
                    row[x] = '.';
                }
                y = target;
                last_turn = x;
            }
        }
    }
    g[start][0] = 'S';
    g[y][w - 1] = 'E';
    decorate(&mut g);
    g
}

fn maze(w: usize, h: usize) -> Vec<Vec<char>> {
    let mut g = vec![vec!['='; w]; h];
    for row in g.iter_mut() {
        row[0] = '#';
        row[w - 1] = '#';
    }
    let mid = h / 2;
    g[mid][0] = 'S';
    g[mid][w - 1] = 'E';
    g
}

/// Cliffs and water well away from the path: scenery that blocks building.
fn decorate(g: &mut [Vec<char>]) {
    let (h, w) = (g.len() as i64, g[0].len() as i64);
    let near_path = |g: &[Vec<char>], x: i64, y: i64| {
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| {
                let (nx, ny) = (x + dx, y + dy);
                nx >= 0
                    && ny >= 0
                    && nx < w
                    && ny < h
                    && matches!(g[ny as usize][nx as usize], '.' | 'S' | 'E')
            })
        })
    };
    for _ in 0..(w * h / 40).max(1) {
        let (x, y) = (rand(w), rand(h));
        let c = if rand(2) == 0 { '^' } else { '~' };
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && nx < w && ny < h && !near_path(g, nx, ny) && rand(3) > 0 {
                g[ny as usize][nx as usize] = c;
            }
        }
    }
}

fn waves(n: i64) -> Value {
    let mut list = Vec::new();
    for i in 0..n {
        let mut w = Value::map();
        if (i + 1) % 10 == 0 {
            w.insert(
                "unit",
                Value::from(BOSSES[((i / 10) as usize) % BOSSES.len()]),
            );
            w.insert("count", Value::Int(1 + i / 20));
            w.insert("interval", Value::Float(2.0));
            w.insert("name", Value::Str(format!("Boss wave {}", i + 1)));
        } else {
            w.insert("unit", Value::from(ROSTER[i as usize % ROSTER.len()]));
            w.insert("count", Value::Int(8 + i / 3));
            w.insert("interval", Value::Float(0.8));
        }
        w.insert("hp_percent", Value::Int(100 + i * 15));
        list.push(w);
    }
    Value::Array(list)
}

fn generate(params: &Value) -> Result<Map, String> {
    let w = params.get("width").i64_or(20).clamp(8, 64) as usize;
    let h = params.get("height").i64_or(11).clamp(5, 40) as usize;
    let lanes = params.get("lanes").i64_or(1).clamp(1, 8) as u8;
    let style = params.get("style").str_or("canyon");
    let grid = if style == "maze" {
        maze(w, h)
    } else {
        canyon(w, h)
    };
    let text: Vec<String> = grid.iter().map(|r| r.iter().collect()).collect();

    let mut extra = BTreeMap::new();
    extra.insert(
        "wave".to_string(),
        waves(params.get("waves").i64_or(15).clamp(1, 100)),
    );
    Ok(Map {
        name: format!(
            "{} {}x{} #{}",
            if style == "maze" { "Field" } else { "Canyon" },
            w,
            h,
            params.get("seed").i64_or(0)
        ),
        modules: None,
        players: lanes,
        teams: None,
        params: Value::Nil,
        legend: BTreeMap::new(),
        lanes: vec![LaneDef {
            owner: LaneOwnerDef::Player,
            grid: text.join("\n"),
        }],
        extra,
    })
}

td_mod_sdk::export_abi!();
td_mod_sdk::export_generator!(generate);
