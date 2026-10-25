//! `td_headless`: run a map without a window.

use std::path::PathBuf;
use std::process::ExitCode;

use td_content::{GameOptions, Library, map_to_toml};
use td_headless::{Bot, Replay, run_with_bots};
use td_mod_api::{TICK_RATE, Value};

const USAGE: &str = "\
usage: td_headless [options]

  --pack-dir PATH     base pack directory or zip (default: packs/base)
  --pack PATH         an extra pack, may repeat
  --list              list maps and generators, then exit
  --map ID            map to play (default: first_steps)
  --generate ID       use a map generator instead of --map
  --param K=V         generator parameter, may repeat
  --export-map PATH   write the (generated) map as TOML and exit
  --players N         override the map's player count
  --seed N            simulation seed (default: 1)
  --minutes N         stop after N game minutes (default: 60)
  --bots              let a greedy bot play every player
  --record PATH       write a replay when the game ends
  --replay PATH       play back a replay and verify its final state
  --quiet             only print the result
";

struct Args {
    pack_dir: PathBuf,
    packs: Vec<PathBuf>,
    list: bool,
    map: String,
    generate: Option<String>,
    params: Value,
    export: Option<PathBuf>,
    players: Option<u8>,
    seed: u64,
    minutes: u64,
    bots: bool,
    record: Option<PathBuf>,
    replay: Option<PathBuf>,
    quiet: bool,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        pack_dir: PathBuf::from("packs/base"),
        packs: Vec::new(),
        list: false,
        map: "first_steps".into(),
        generate: None,
        params: Value::map(),
        export: None,
        players: None,
        seed: 1,
        minutes: 60,
        bots: false,
        record: None,
        replay: None,
        quiet: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--pack-dir" => a.pack_dir = val()?.into(),
            "--pack" => a.packs.push(val()?.into()),
            "--list" => a.list = true,
            "--map" => a.map = val()?,
            "--generate" => a.generate = Some(val()?),
            "--param" => {
                let v = val()?;
                let (k, v) = v.split_once('=').ok_or("--param wants K=V")?;
                let value = v
                    .parse::<i64>()
                    .map(Value::Int)
                    .or_else(|_| v.parse::<f64>().map(Value::Float))
                    .unwrap_or_else(|_| Value::from(v));
                a.params.insert(k, value);
            }
            "--export-map" => a.export = Some(val()?.into()),
            "--players" => {
                a.players = Some(val()?.parse().map_err(|_| "--players wants a number")?)
            }
            "--seed" => a.seed = val()?.parse().map_err(|_| "--seed wants a number")?,
            "--minutes" => a.minutes = val()?.parse().map_err(|_| "--minutes wants a number")?,
            "--bots" => a.bots = true,
            "--record" => a.record = Some(val()?.into()),
            "--replay" => a.replay = Some(val()?.into()),
            "--quiet" => a.quiet = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn real_main() -> Result<bool, String> {
    let args = parse()?;
    let lib = Library::open(&args.pack_dir, &args.packs)?;

    if args.list {
        for m in &lib.maps {
            println!(
                "map {}:{}  \"{}\"  players={}",
                m.pack, m.id, m.map.name, m.map.players
            );
        }
        for g in &lib.generators {
            let params: Vec<&str> = g.def.params.iter().map(|p| p.name.as_str()).collect();
            println!("generator {}  ({})", g.def.id, params.join(", "));
        }
        return Ok(true);
    }

    if let Some(path) = &args.replay {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let replay = Replay::from_bytes(&bytes)?;
        let mut game = td_content::start(
            &lib,
            &replay.map,
            &GameOptions {
                players: Some(replay.players),
                seed: replay.seed,
            },
        )?;
        let ok = replay.play(&mut game);
        println!(
            "replay of \"{}\": {} ticks, {}",
            replay.map.name,
            game.sim.tick,
            if ok { "state matches" } else { "STATE DIFFERS" }
        );
        return Ok(ok);
    }

    let map = match &args.generate {
        Some(id) => lib.generate(id, &args.params)?,
        None => lib
            .find_map(&args.map)
            .ok_or_else(|| format!("no map `{}` (try --list)", args.map))?
            .map
            .clone(),
    };
    if let Some(path) = &args.export {
        std::fs::write(path, map_to_toml(&map)).map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
        return Ok(true);
    }

    let mut game = td_content::start(
        &lib,
        &map,
        &GameOptions {
            players: args.players,
            seed: args.seed,
        },
    )?;
    let mut bots: Vec<Bot> = if args.bots {
        game.sim.players.iter().map(|p| Bot::new(p.id)).collect()
    } else {
        Vec::new()
    };
    if !args.quiet {
        let rules: Vec<&str> = game.rules().map(|(n, _)| n).collect();
        println!(
            "\"{}\": {} player(s), {} lane(s), rules: {}",
            map.name,
            game.sim.players.len(),
            game.sim.lanes.len(),
            rules.join(", ")
        );
    }
    let quiet = args.quiet;
    let result = run_with_bots(
        &mut game,
        &mut bots,
        args.minutes * 60 * TICK_RATE as u64,
        |tick, _, text| {
            if !quiet {
                let secs = tick / TICK_RATE as u64;
                println!("[{:02}:{:02}] {text}", secs / 60, secs % 60);
            }
        },
    );
    for line in game.sim.log.drain(..) {
        eprintln!("log: {line}");
    }
    let secs = game.sim.tick / TICK_RATE as u64;
    match &result {
        Some(r) => println!(
            "result after {:02}:{:02}: {}  winners={:?} losers={:?}",
            secs / 60,
            secs % 60,
            if r.text.is_empty() {
                "game over"
            } else {
                &r.text
            },
            r.winners,
            r.losers
        ),
        None => println!("no result after {:02}:{:02}", secs / 60, secs % 60),
    }
    for p in &game.sim.players {
        let res: Vec<String> = game
            .sim
            .resources(td_mod_api::Scope::Player(p.id))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let mut built: std::collections::BTreeMap<&str, u32> = Default::default();
        for s in game.sim.structures.values().filter(|s| s.owner == p.id) {
            *built
                .entry(game.sim.content.structures[s.kind].id.as_str())
                .or_default() += 1;
        }
        let built: Vec<String> = built.iter().map(|(k, n)| format!("{k} x{n}")).collect();
        println!("  {}: {}  [{}]", p.name, res.join(" "), built.join(", "));
    }
    println!("state hash {:016x}", game.sim.state_hash());
    if let Some(path) = &args.record {
        std::fs::write(path, Replay::record(&game, &map, args.seed).to_bytes())
            .map_err(|e| e.to_string())?;
        println!("replay written to {}", path.display());
    }
    Ok(true)
}

fn main() -> ExitCode {
    match real_main() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
