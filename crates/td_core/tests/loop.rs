//! A Rust-side rule proves the event → command loop without wasm.

use std::sync::Arc;

use td_core::content::{DamageTable, StructureKind, TileKind, UnitKind};
use td_core::{Content, Game, LaneSpec, Rule, Sim, SimSetup, TileFlags};
use td_mod_api::*;

fn toml_unit(s: &str) -> UnitKind {
    toml::from_str(s).unwrap()
}
fn toml_structure(s: &str) -> StructureKind {
    toml::from_str(s).unwrap()
}

fn content() -> Arc<Content> {
    let tiles = vec![
        TileKind {
            id: "path".into(),
            name: None,
            flags: TileFlags::GROUND | TileFlags::AIR | TileFlags::UNIT,
            sprite: None,
            color: None,
        },
        TileKind {
            id: "field".into(),
            name: None,
            flags: TileFlags::GROUND | TileFlags::AIR | TileFlags::UNIT | TileFlags::BUILD,
            sprite: None,
            color: None,
        },
        TileKind {
            id: "grass".into(),
            name: None,
            flags: TileFlags::AIR | TileFlags::UNIT | TileFlags::BUILD,
            sprite: None,
            color: None,
        },
    ];
    let units = vec![
        toml_unit("id = 'grunt'\nhp = 50\nspeed = 2.0\nbounty = { gold = 1 }"),
        toml_unit("id = 'bat'\nhp = 20\nspeed = 3.0\nmoves_on = ['air']"),
    ];
    let structures = vec![
        toml_structure(
            "id = 'arrow'\ncost = { gold = 10 }\nrange = 3\ndamage = 10\ncooldown = 0.5\ntargets = ['ground', 'air']",
        ),
        toml_structure("id = 'wall'\ncost = { gold = 1 }"),
    ];
    Arc::new(Content::new(tiles, units, structures, DamageTable::default()).unwrap())
}

/// 12x5 lane: spawn at the left, exit at the right, an open field between.
fn lane() -> LaneSpec {
    let (w, h) = (12u32, 5u32);
    let mut tiles = vec![1usize; (w * h) as usize];
    for y in 0..h {
        tiles[(y * w) as usize] = 0;
        tiles[(y * w + w - 1) as usize] = 0;
    }
    LaneSpec {
        owner: LaneOwner::Player(0),
        width: w,
        height: h,
        tiles,
        spawns: vec![(0, 2)],
        exits: vec![(11, 2)],
    }
}

fn sim(content: Arc<Content>) -> Sim {
    Sim::new(
        content,
        SimSetup {
            players: vec![PlayerInfo {
                id: 0,
                team: 0,
                name: "p0".into(),
            }],
            lanes: vec![lane()],
            seed: 7,
            default_sell_value: 0.5,
        },
    )
    .unwrap()
}

#[derive(Default)]
struct TestRule {
    lives: i64,
    leaked: u32,
    killed: u32,
    started: bool,
}

impl Rule for TestRule {
    fn name(&self) -> &str {
        "test"
    }
    fn on_event(&mut self, event: &Event, out: &mut Vec<Command>) -> Result<(), String> {
        match event {
            Event::GameStart { .. } => {
                self.started = true;
                out.push(Command::AddResource {
                    scope: Scope::Player(0),
                    name: "gold".into(),
                    delta: 100,
                    allow_negative: false,
                });
                out.push(Command::AddResource {
                    scope: Scope::Player(0),
                    name: "lives".into(),
                    delta: 3,
                    allow_negative: false,
                });
                out.push(Command::SetTimer {
                    id: 1,
                    ticks: 15,
                    repeat: true,
                });
            }
            Event::Timer { id: 1 } if self.killed + self.leaked < 10 => {
                out.push(Command::Spawn {
                    kind: "grunt".into(),
                    lane: 0,
                    owner: NEUTRAL,
                    hp_percent: 100,
                    at: None,
                    idle: false,
                });
            }
            Event::UnitReachedExit { .. } => {
                self.leaked += 1;
                out.push(Command::AddResource {
                    scope: Scope::Player(0),
                    name: "lives".into(),
                    delta: -1,
                    allow_negative: true,
                });
            }
            Event::UnitDied { .. } => self.killed += 1,
            Event::ResourceChanged { name, value, .. } if name == "lives" => {
                self.lives = *value;
                if *value <= 0 {
                    out.push(Command::EndGame {
                        result: GameResult {
                            winners: vec![],
                            losers: vec![0],
                            text: "lost".into(),
                        },
                    });
                }
            }
            _ => {}
        }
        if self.killed + self.leaked == 10 {
            out.push(Command::EndGame {
                result: GameResult {
                    winners: vec![0],
                    losers: vec![],
                    text: "won".into(),
                },
            });
        }
        Ok(())
    }
}

#[test]
fn undefended_lane_loses() {
    let mut game = Game::new(sim(content()), vec![Box::new(TestRule::default())]);
    game.run(30 * 60);
    assert_eq!(game.sim.result().unwrap().losers, vec![0]);
}

#[test]
fn towers_win_and_replay_is_identical() {
    let run = || {
        let mut game = Game::new(sim(content()), vec![Box::new(TestRule::default())]);
        game.step();
        game.step();
        for (x, y) in [(3, 1), (6, 3), (9, 1)] {
            game.queue_player(
                0,
                Command::Build {
                    kind: "arrow".into(),
                    at: Cell::new(0, x, y),
                    owner: 0,
                    builder: None,
                },
            );
        }
        game.run(30 * 60);
        (
            game.sim.result().cloned(),
            game.sim.state_hash(),
            game.sim.resource(Scope::Player(0), "gold"),
        )
    };
    let (a, ha, gold) = run();
    let (b, hb, _) = run();
    assert_eq!(a.as_ref().unwrap().winners, vec![0], "{a:?}");
    assert_eq!(a, b);
    assert_eq!(ha, hb);
    assert_eq!(gold, 70);
}

#[test]
fn blocking_placement_is_refused() {
    let mut sim = sim(content());
    sim.queue(
        td_core::Source::System,
        Command::AddResource {
            scope: Scope::Player(0),
            name: "gold".into(),
            delta: 100,
            allow_negative: false,
        },
    );
    sim.step();
    // A full column of walls would cut the lane.
    for y in 0..4 {
        sim.queue(
            td_core::Source::Player(0),
            Command::Build {
                kind: "wall".into(),
                at: Cell::new(0, 5, y),
                owner: 0,
                builder: None,
            },
        );
    }
    sim.step();
    assert_eq!(sim.structures.len(), 4);
    let wall = sim.content.structure("wall").unwrap();
    assert!(sim.check_placement(wall, Cell::new(0, 5, 4), 0).is_err());
    // Flyers ignore walls: the air field still reaches the spawn.
    let lane = &sim.lanes[0];
    assert!(lane.dist(TileFlags::AIR, 0, 2) < lane.dist(TileFlags::GROUND, 0, 2));
}
