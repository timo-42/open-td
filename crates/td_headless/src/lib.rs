//! Run open-td without a window: tests, balance runs, bots and replays.

pub mod bot;
pub mod replay;

use td_core::Game;
use td_mod_api::{GameResult, MessageTarget};

pub use bot::Bot;
pub use replay::Replay;

/// Steps `game` with `bots` playing until it ends or `max_ticks` pass.
/// Calls `on_message` for every HUD message.
pub fn run_with_bots(
    game: &mut Game,
    bots: &mut [Bot],
    max_ticks: u64,
    mut on_message: impl FnMut(u64, &MessageTarget, &str),
) -> Option<GameResult> {
    let end = game.sim.tick + max_ticks;
    while !game.is_over() && game.sim.tick < end {
        for bot in bots.iter_mut() {
            for c in bot.think(&mut game.sim) {
                game.queue_player(bot.player, c);
            }
        }
        game.step();
        for (to, text) in game.sim.messages.drain(..) {
            on_message(game.sim.tick, &to, &text);
        }
        game.sim.fx.clear();
    }
    game.sim.result().cloned()
}
