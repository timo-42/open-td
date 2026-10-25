//! Unattended runs: `--autoplay` puts a bot in the local seat, `--speed`
//! fast-forwards, `--screenshot PATH --after SECS` saves the window and
//! quits. Used for smoke tests and screenshots.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use td_headless::Bot;

use crate::session::Session;
use crate::{Options, Screen};

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(Screen::Playing), setup)
        .add_systems(Update, screenshot);
}

fn setup(options: Res<Options>, mut s: ResMut<Session>) {
    if options.autoplay {
        let me = s.me;
        s.bots.push(Bot::new(me));
    }
    if let Some(speed) = options.speed {
        s.speed = speed;
    }
}

fn screenshot(
    mut commands: Commands,
    time: Res<Time>,
    options: Res<Options>,
    mut done: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &options.screenshot else {
        return;
    };
    let t = time.elapsed_secs();
    if !*done && t >= options.after {
        *done = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if t >= options.after + 1.5 {
        exit.write(AppExit::Success);
    }
}
