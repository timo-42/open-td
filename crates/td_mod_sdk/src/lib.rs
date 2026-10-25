//! Helpers for writing open-td rule modules in Rust.
//!
//! ```ignore
//! use td_mod_sdk::prelude::*;
//!
//! #[derive(Default)]
//! struct MyRules { /* state lives in wasm memory */ }
//!
//! impl Module for MyRules {
//!     fn init(&mut self, init: ModuleInit) {}
//!     fn on_event(&mut self, event: Event, out: &mut Vec<Command>) {}
//! }
//!
//! td_mod_sdk::export_module!(MyRules);
//! ```
//!
//! Build with `cargo build --target wasm32-unknown-unknown` as a `cdylib`.

pub use td_mod_api as api;
pub use td_mod_api::codec;

pub mod prelude {
    pub use crate::{Module, log, random, random_below};
    pub use td_mod_api::*;
}

use td_mod_api::{Command, Event, ModuleInit};

/// A rule module. One instance lives for the whole game.
pub trait Module: Default + 'static {
    fn init(&mut self, _init: ModuleInit) {}
    fn on_event(&mut self, _event: Event, _out: &mut Vec<Command>) {}
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "td")]
unsafe extern "C" {
    #[link_name = "log"]
    fn host_log(ptr: *const u8, len: u32);
    #[link_name = "random"]
    fn host_random() -> i64;
}

/// Writes a line to the host log.
pub fn log(text: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        host_log(text.as_ptr(), text.len() as u32)
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = text;
}

/// A deterministic random number from the game's seeded generator.
pub fn random() -> u64 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        host_random() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

/// A deterministic random number in `0..n` (`0` when `n == 0`).
pub fn random_below(n: u64) -> u64 {
    if n == 0 { 0 } else { random() % n }
}

#[doc(hidden)]
pub mod rt {
    use super::*;

    pub fn alloc(len: u32) -> u32 {
        let buf = vec![0u8; len as usize].into_boxed_slice();
        Box::into_raw(buf) as *mut u8 as usize as u32
    }

    /// # Safety
    /// `ptr` and `len` must come from [`alloc`] or an output of this module.
    pub unsafe fn free(ptr: u32, len: u32) {
        if ptr != 0 {
            let slice = core::ptr::slice_from_raw_parts_mut(ptr as usize as *mut u8, len as usize);
            drop(unsafe { Box::from_raw(slice) });
        }
    }

    /// Takes ownership of an input buffer written by the host.
    ///
    /// # Safety
    /// Same as [`free`].
    pub unsafe fn take_input(ptr: u32, len: u32) -> Box<[u8]> {
        let slice = core::ptr::slice_from_raw_parts_mut(ptr as usize as *mut u8, len as usize);
        unsafe { Box::from_raw(slice) }
    }

    pub fn output(bytes: Vec<u8>) -> i64 {
        if bytes.is_empty() {
            return 0;
        }
        let len = bytes.len() as u64;
        let ptr = Box::into_raw(bytes.into_boxed_slice()) as *mut u8 as usize as u64;
        ((ptr << 32) | len) as i64
    }

    pub fn decode_or_log<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
        match td_mod_api::codec::decode(bytes) {
            Ok(v) => Some(v),
            Err(e) => {
                log(&format!("decode error: {e}"));
                None
            }
        }
    }
}

/// Exports `td_abi_version`, `td_alloc` and `td_free`. Included by the other
/// export macros; use it directly only in a module with custom exports.
#[macro_export]
macro_rules! export_abi {
    () => {
        #[unsafe(no_mangle)]
        pub extern "C" fn td_abi_version() -> u32 {
            $crate::api::ABI_VERSION
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn td_alloc(len: u32) -> u32 {
            $crate::rt::alloc(len)
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn td_free(ptr: u32, len: u32) {
            unsafe { $crate::rt::free(ptr, len) }
        }
    };
}

/// Exports a rule module: `td_init` and `td_on_event` backed by one
/// instance of `$ty: Module`.
#[macro_export]
macro_rules! export_module {
    ($ty:ty) => {
        $crate::export_abi!();

        thread_local! {
            static TD_MODULE: ::core::cell::RefCell<$ty> =
                ::core::cell::RefCell::new(<$ty as ::core::default::Default>::default());
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn td_init(ptr: u32, len: u32) -> i64 {
            let input = unsafe { $crate::rt::take_input(ptr, len) };
            let Some(init) = $crate::rt::decode_or_log::<$crate::api::ModuleInit>(&input) else {
                return -1;
            };
            TD_MODULE.with(|m| $crate::Module::init(&mut *m.borrow_mut(), init));
            0
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn td_on_event(ptr: u32, len: u32) -> i64 {
            let input = unsafe { $crate::rt::take_input(ptr, len) };
            let Some(event) = $crate::rt::decode_or_log::<$crate::api::Event>(&input) else {
                return -1;
            };
            let mut out = ::std::vec::Vec::new();
            TD_MODULE.with(|m| $crate::Module::on_event(&mut *m.borrow_mut(), event, &mut out));
            if out.is_empty() {
                return 0;
            }
            match $crate::codec::encode(&out) {
                Ok(bytes) => $crate::rt::output(bytes),
                Err(_) => -1,
            }
        }
    };
}

/// Exports a map generator: `td_generate_map` calling
/// `fn(&Value) -> Result<Map, String>`.
#[macro_export]
macro_rules! export_generator {
    ($f:path) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn td_generate_map(ptr: u32, len: u32) -> i64 {
            let input = unsafe { $crate::rt::take_input(ptr, len) };
            let Some(params) = $crate::rt::decode_or_log::<$crate::api::Value>(&input) else {
                return -1;
            };
            match $f(&params) {
                Ok(map) => match $crate::codec::encode(&map) {
                    Ok(bytes) => $crate::rt::output(bytes),
                    Err(_) => -1,
                },
                Err(e) => {
                    $crate::log(&e);
                    -1
                }
            }
        }
    };
}
