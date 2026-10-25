//! MessagePack encoding used on the module boundary.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(value).map_err(|e| e.to_string())
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    rmp_serde::from_slice(bytes).map_err(|e| e.to_string())
}
