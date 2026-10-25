//! open-td client: top-down 2D rendering of the simulation, input, HUD,
//! map and generator selection. The simulation runs in-process at a fixed
//! 30 Hz; everything here only reads it and queues player commands.

// Bevy queries are long types by nature.
#![allow(clippy::type_complexity)]

mod assets;
mod debug;
mod hud;
mod input;
mod menu;
mod session;
mod style;
mod vfx;
mod world;

use bevy::prelude::*;
use bevy::window::WindowResolution;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Menu,
    Playing,
}

/// Command-line options (native only).
#[derive(Resource, Debug, Clone, Default)]
pub struct Options {
    pub pack_dir: Option<std::path::PathBuf>,
    pub packs: Vec<std::path::PathBuf>,
    /// Start this map right away.
    pub map: Option<String>,
    pub players: Option<u8>,
    pub seed: u64,
    /// A bot plays the local seat too.
    pub autoplay: bool,
    pub speed: Option<f32>,
    /// Save a screenshot here after `after` seconds, then quit.
    pub screenshot: Option<std::path::PathBuf>,
    pub after: f32,
}

fn options() -> Result<Options, String> {
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut o = Options {
        seed: 1,
        after: 5.0,
        ..Default::default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            let mut val = || it.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--pack-dir" => o.pack_dir = Some(val()?.into()),
                "--pack" => o.packs.push(val()?.into()),
                "--map" => o.map = Some(val()?),
                "--players" => {
                    o.players = Some(val()?.parse().map_err(|_| "--players wants a number")?)
                }
                "--seed" => o.seed = val()?.parse().map_err(|_| "--seed wants a number")?,
                "--autoplay" => o.autoplay = true,
                "--speed" => o.speed = Some(val()?.parse().map_err(|_| "--speed wants a number")?),
                "--screenshot" => o.screenshot = Some(val()?.into()),
                "--after" => o.after = val()?.parse().map_err(|_| "--after wants seconds")?,
                "-h" | "--help" => {
                    return Err("usage: open-td [--pack-dir DIR|ZIP] [--pack ZIP]... [--map ID] [--players N] [--seed N]\n               [--autoplay] [--speed X] [--screenshot PNG --after SECS]".into());
                }
                other => return Err(format!("unknown option {other}")),
            }
        }
    }
    Ok(o)
}

fn main() {
    let options = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "open-td".into(),
                resolution: WindowResolution::new(1280, 800),
                canvas: Some("#open-td".into()),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(style::BACKDROP))
        .insert_resource(options)
        .init_state::<Screen>()
        .add_plugins((
            assets::plugin,
            style::plugin,
            menu::plugin,
            session::plugin,
            world::plugin,
            input::plugin,
            hud::plugin,
            vfx::plugin,
            debug::plugin,
        ))
        .run();
}
