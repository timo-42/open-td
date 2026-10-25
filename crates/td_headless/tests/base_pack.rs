//! End-to-end tests against the real base pack and its wasm modules.
//! Needs `cargo xtask modules` first (CI does this).

use std::path::Path;

use td_content::{GameOptions, Library};
use td_headless::{Bot, Replay, run_with_bots};
use td_mod_api::{Command, LaneOwner, NEUTRAL, Scope, TICK_RATE, Value};

fn lib() -> Library {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/base");
    assert!(
        base.join("modules/waves.wasm").exists(),
        "base modules are not built: run `cargo xtask modules` first"
    );
    Library::open(&base, &[]).expect("base pack loads")
}

const MINUTES: u64 = 60 * TICK_RATE as u64;

#[test]
fn every_base_map_starts() {
    let lib = lib();
    assert!(lib.maps.len() >= 5);
    for m in &lib.maps {
        let mut game = td_content::start(&lib, &m.map, &GameOptions::default())
            .unwrap_or_else(|e| panic!("{}: {e}", m.id));
        for _ in 0..TICK_RATE * 30 {
            game.step();
        }
        assert!(
            game.rules().all(|(_, enabled)| enabled),
            "{}: a module was disabled: {:?}",
            m.id,
            game.sim.log
        );
        assert!(
            !game.sim.units.is_empty(),
            "{}: nothing spawned in 30 s",
            m.id
        );
    }
}

#[test]
fn undefended_map_is_lost() {
    let lib = lib();
    let map = &lib.find_map("first_steps").unwrap().map;
    let mut game = td_content::start(&lib, map, &GameOptions::default()).unwrap();
    let result = run_with_bots(&mut game, &mut [], 10 * MINUTES, |_, _, _| {}).expect("game ends");
    assert_eq!(result.losers, vec![0]);
    assert_eq!(game.sim.resource(Scope::Player(0), "lives"), 0);
}

#[test]
fn bot_wins_first_steps() {
    let lib = lib();
    let map = &lib.find_map("first_steps").unwrap().map;
    let mut game = td_content::start(&lib, map, &GameOptions::default()).unwrap();
    let mut bots = [Bot::new(0)];
    let result =
        run_with_bots(&mut game, &mut bots, 30 * MINUTES, |_, _, _| {}).expect("game ends");
    assert_eq!(result.winners, vec![0], "{result:?}");
    // Builders are required by default, so every structure came via one.
    assert!(game.sim.requires_builder(0));
    assert!(game.sim.structures.len() >= 5);
}

#[test]
fn same_seed_same_game_and_replay_matches() {
    let lib = lib();
    let map = lib.find_map("open_field").unwrap().map.clone();
    let play = || {
        let mut game = td_content::start(
            &lib,
            &map,
            &GameOptions {
                players: None,
                seed: 42,
            },
        )
        .unwrap();
        let mut bots = [Bot::new(0)];
        run_with_bots(&mut game, &mut bots, 4 * MINUTES, |_, _, _| {});
        game
    };
    let a = play();
    let b = play();
    assert_eq!(a.sim.state_hash(), b.sim.state_hash());
    assert!(!a.sim.command_log.is_empty());

    let replay = Replay::from_bytes(&Replay::record(&a, &map, 42).to_bytes()).unwrap();
    let mut c = td_content::start(
        &lib,
        &replay.map,
        &GameOptions {
            players: Some(replay.players),
            seed: replay.seed,
        },
    )
    .unwrap();
    assert!(replay.play(&mut c), "replay diverged");
}

#[test]
fn generator_is_deterministic_and_playable() {
    let lib = lib();
    let mut params = Value::map();
    params.insert("seed", Value::Int(7));
    params.insert("lanes", Value::Int(2));
    let a = lib.generate("random_canyon", &params).unwrap();
    let b = lib.generate("random_canyon", &params).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.players, 2);
    params.insert("seed", Value::Int(8));
    assert_ne!(
        lib.generate("random_canyon", &params).unwrap().lanes,
        a.lanes
    );
    for style in ["canyon", "maze"] {
        for seed in 0..20 {
            params.insert("seed", Value::Int(seed));
            params.insert("style", Value::from(style));
            let map = lib.generate("random_canyon", &params).unwrap();
            let game = td_content::start(&lib, &map, &GameOptions::default());
            assert!(
                game.is_ok(),
                "{style} seed {seed}: {:?}\n{}",
                game.err(),
                map.lanes[0].grid
            );
        }
    }
    // Round trip through TOML.
    let text = td_content::map_to_toml(&a);
    assert_eq!(td_content::map_from_toml(&text).unwrap(), a);
}

#[test]
fn versus_sends_into_the_opponent_lane() {
    let lib = lib();
    let map = &lib.find_map("duel").unwrap().map;
    let mut game = td_content::start(&lib, map, &GameOptions::default()).unwrap();
    game.step();
    game.step();
    let gold = game.sim.resource(Scope::Player(0), "gold");
    let mut data = Value::map();
    data.insert("unit", Value::from("wolf"));
    game.queue_player(
        0,
        Command::Request {
            request: "send_monster".into(),
            data,
        },
    );
    for _ in 0..4 {
        game.step();
    }
    let sent: Vec<_> = game.sim.units.values().filter(|u| u.owner == 0).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        game.sim.lanes[sent[0].lane as usize].owner,
        LaneOwner::Player(1)
    );
    assert_eq!(game.sim.resource(Scope::Player(0), "gold"), gold - 18);
    assert!(
        game.sim
            .units
            .values()
            .all(|u| u.owner == 0 || u.owner == NEUTRAL)
    );
}

#[test]
fn builder_walks_then_builds() {
    let lib = lib();
    let map = &lib.find_map("first_steps").unwrap().map;
    let mut game = td_content::start(&lib, map, &GameOptions::default()).unwrap();
    game.step();
    game.step();
    let builder = game
        .sim
        .units
        .values()
        .find(|u| u.owner == 0)
        .map(|u| u.id)
        .expect("a builder");
    let at = td_mod_api::Cell::new(0, 8, 3);
    // Without a builder the rules refuse.
    game.queue_player(
        0,
        Command::Build {
            kind: "arrow_tower".into(),
            at,
            owner: 0,
            builder: None,
        },
    );
    game.step();
    assert!(game.sim.structures.is_empty());
    assert!(game.sim.messages.iter().any(|(_, t)| t.contains("builder")));
    game.queue_player(
        0,
        Command::Build {
            kind: "arrow_tower".into(),
            at,
            owner: 0,
            builder: Some(builder),
        },
    );
    game.step();
    assert!(game.sim.structures.is_empty(), "builder must walk first");
    for _ in 0..TICK_RATE * 10 {
        game.step();
    }
    assert_eq!(game.sim.structures.len(), 1);
    assert_eq!(game.sim.structures.values().next().unwrap().cell(), at);

    // buy_builder spends gold and adds a second builder.
    game.queue_player(
        0,
        Command::Request {
            request: "buy_builder".into(),
            data: Value::Nil,
        },
    );
    for _ in 0..3 {
        game.step();
    }
    let builders = game.sim.units.values().filter(|u| u.owner == 0).count();
    assert_eq!(builders, 2);
}

#[test]
fn builder_queues_orders() {
    let lib = lib();
    let map = &lib.find_map("first_steps").unwrap().map;
    let mut game = td_content::start(&lib, map, &GameOptions::default()).unwrap();
    game.step();
    game.step();
    let builder = game
        .sim
        .units
        .values()
        .find(|u| u.owner == 0)
        .map(|u| u.id)
        .unwrap();
    for x in [6, 8, 10] {
        let at = td_mod_api::Cell::new(0, x, 3);
        game.queue_player(
            0,
            Command::Build {
                kind: "arrow_tower".into(),
                at,
                owner: 0,
                builder: Some(builder),
            },
        );
    }
    for _ in 0..TICK_RATE * 15 {
        game.step();
    }
    assert_eq!(game.sim.structures.len(), 3, "{:?}", game.sim.messages);
}
