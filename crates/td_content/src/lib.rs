//! Packs: `info.toml`, stat tables, maps, zip reading, validation, and
//! turning a map into a running [`td_core::Game`].

pub mod files;
pub mod map;
pub mod pack;
pub mod session;
pub mod value;

pub use files::PackFiles;
pub use map::{map_from_toml, map_to_toml};
pub use pack::{GeneratorDef, Library, MapEntry, PackMeta, VfxPreset};
pub use session::{GameOptions, ResolvedMap, resolve, start};
