//! Replays: the map, the seed and every player command. Module commands
//! are not stored; they follow deterministically.

use serde::{Deserialize, Serialize};
use td_core::Game;
use td_mod_api::{Command, Map, PlayerId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub version: u32,
    pub map: Map,
    pub players: u8,
    pub seed: u64,
    pub ticks: u64,
    /// `Sim::state_hash` after `ticks`.
    pub hash: u64,
    pub commands: Vec<(u64, PlayerId, Command)>,
}

impl Replay {
    pub fn record(game: &Game, map: &Map, seed: u64) -> Replay {
        Replay {
            version: 1,
            map: map.clone(),
            players: game.sim.players.len() as u8,
            seed,
            ticks: game.sim.tick,
            hash: game.sim.state_hash(),
            commands: game
                .sim
                .command_log
                .iter()
                .map(|c| (c.tick, c.player, c.command.clone()))
                .collect(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        td_mod_api::codec::encode(self).expect("replay encodes")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Replay, String> {
        td_mod_api::codec::decode(bytes)
    }

    /// Feeds the recorded commands into `game` while stepping it to the
    /// recorded tick. Returns whether the final state hash matches.
    pub fn play(&self, game: &mut Game) -> bool {
        let mut next = 0;
        while game.sim.tick < self.ticks {
            while let Some((tick, player, cmd)) = self.commands.get(next) {
                if *tick != game.sim.tick {
                    break;
                }
                game.queue_player(*player, cmd.clone());
                next += 1;
            }
            game.step();
            if game.is_over() {
                break;
            }
        }
        game.sim.state_hash() == self.hash
    }
}
