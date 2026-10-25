use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Tile flags. A tile kind is a set of these; a unit's `moves_on`, a
/// structure's `targets` and `blocks` are masks of them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TileFlags(pub u8);

impl TileFlags {
    pub const NONE: TileFlags = TileFlags(0);
    /// Ground units may walk here.
    pub const GROUND: TileFlags = TileFlags(1);
    /// Flying units may move here.
    pub const AIR: TileFlags = TileFlags(2);
    /// Player-controlled units (builders, heroes) may walk here.
    pub const UNIT: TileFlags = TileFlags(4);
    /// Structures may be placed here.
    pub const BUILD: TileFlags = TileFlags(8);

    const NAMES: [(&'static str, TileFlags); 4] = [
        ("ground", TileFlags::GROUND),
        ("air", TileFlags::AIR),
        ("unit", TileFlags::UNIT),
        ("build", TileFlags::BUILD),
    ];

    pub fn contains(self, other: TileFlags) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn intersects(self, other: TileFlags) -> bool {
        self.0 & other.0 != 0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn parse(name: &str) -> Option<TileFlags> {
        Self::NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| *f)
    }
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(_, f)| self.intersects(*f))
            .map(|(n, _)| *n)
            .collect()
    }
}

impl std::ops::BitOr for TileFlags {
    type Output = TileFlags;
    fn bitor(self, rhs: TileFlags) -> TileFlags {
        TileFlags(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for TileFlags {
    type Output = TileFlags;
    fn bitand(self, rhs: TileFlags) -> TileFlags {
        TileFlags(self.0 & rhs.0)
    }
}
impl std::ops::Not for TileFlags {
    type Output = TileFlags;
    fn not(self) -> TileFlags {
        TileFlags(!self.0 & 0x0f)
    }
}

impl fmt::Debug for TileFlags {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.names())
    }
}

impl Serialize for TileFlags {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.names().serialize(s)
    }
}

impl<'de> Deserialize<'de> for TileFlags {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(d)?;
        let mut out = TileFlags::NONE;
        for n in names {
            out = out
                | TileFlags::parse(&n).ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "unknown tile flag `{n}` (expected ground, air, unit, build)"
                    ))
                })?;
        }
        Ok(out)
    }
}
