//! open-td engine kernel.
//!
//! Lanes, tiles with flag masks, pathing, units, structures, combat,
//! players, teams, named resources, timers, events and commands. It knows
//! nothing about lives, waves, gold or winning: those are rule modules
//! talking to it through [`td_mod_api::Event`] and [`td_mod_api::Command`].

pub mod content;
pub mod flags;
pub mod game;
pub mod lane;
pub mod rng;
pub mod sim;

pub use content::Content;
pub use flags::TileFlags;
pub use game::{Game, Rule};
pub use lane::{Lane, LaneSpec};
pub use sim::{Fx, Sim, SimSetup, Source};
pub use td_mod_api as api;
