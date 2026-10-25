//! Fuel, traps and the ABI handshake, with hand-written wat modules.

use td_core::Rule;
use td_mod_api::Event;
use td_mods::WasmModule;

const PRELUDE: &str = r#"
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "td_abi_version") (result i32) (i32.const 1))
  (func (export "td_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $p))
  (func (export "td_free") (param i32 i32))
"#;

fn module(body: &str) -> Vec<u8> {
    wat::parse_str(format!("(module {PRELUDE} {body})")).unwrap()
}

fn tick() -> Event {
    Event::Tick { n: 1 }
}

#[test]
fn runaway_module_runs_out_of_fuel() {
    let wasm = module(
        r#"(func (export "td_on_event") (param i32 i32) (result i64) (loop $l (br $l)) (i64.const 0))"#,
    );
    let mut m = WasmModule::load("spin", &wasm, 0).unwrap();
    let mut out = Vec::new();
    let err = m.on_event(&tick(), &mut out).unwrap_err();
    assert!(err.contains("out of fuel"), "{err}");
}

#[test]
fn trap_is_an_error() {
    let wasm = module(r#"(func (export "td_on_event") (param i32 i32) (result i64) unreachable)"#);
    let mut m = WasmModule::load("trap", &wasm, 0).unwrap();
    assert!(m.on_event(&tick(), &mut Vec::new()).is_err());
}

#[test]
fn garbage_output_is_an_error() {
    // Returns 4 bytes at address 0 that are not a command list.
    let wasm = module(
        r#"(data (i32.const 0) "\c1\c1\c1\c1")
           (func (export "td_on_event") (param i32 i32) (result i64) (i64.const 4))"#,
    );
    let mut m = WasmModule::load("junk", &wasm, 0).unwrap();
    assert!(m.on_event(&tick(), &mut Vec::new()).is_err());
}

#[test]
fn wrong_abi_version_is_refused() {
    let wasm = wat::parse_str(
        r#"(module (memory (export "memory") 1)
             (func (export "td_abi_version") (result i32) (i32.const 99))
             (func (export "td_alloc") (param i32) (result i32) (i32.const 0))
             (func (export "td_free") (param i32 i32)))"#,
    )
    .unwrap();
    let err = WasmModule::load("future", &wasm, 0).err().unwrap();
    assert!(err.contains("ABI version 99"), "{err}");
}
