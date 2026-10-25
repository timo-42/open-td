//! Runs rule modules and map generators with wasmi on every target.
//!
//! Every call gets a fuel budget. A module that traps, runs out of fuel,
//! returns garbage or grows its memory past the limit is disabled; the
//! engine tells the players. Because fuel is counted per instruction, a
//! runaway module stops at the same point on every machine.

use td_core::Rule;
use td_core::rng::Rng;
use td_mod_api::codec::{decode, encode};
use td_mod_api::{ABI_VERSION, Command, Event, Map, ModuleInit, Value};
use wasmi::{
    Caller, Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc,
};

/// Fuel for `td_init` and `td_generate_map`.
pub const SETUP_FUEL: u64 = 200_000_000;
/// Fuel for one `td_on_event` call.
pub const EVENT_FUEL: u64 = 10_000_000;
/// Linear memory cap per module.
pub const MEMORY_LIMIT: usize = 64 << 20;

struct Host {
    name: String,
    rng: Rng,
    limits: StoreLimits,
    logs: Vec<String>,
}

/// A loaded wasm module instance.
pub struct WasmModule {
    name: String,
    store: Store<Host>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    free: TypedFunc<(u32, u32), ()>,
    init: Option<TypedFunc<(u32, u32), i64>>,
    on_event: Option<TypedFunc<(u32, u32), i64>>,
    generate: Option<TypedFunc<(u32, u32), i64>>,
    /// Fuel per `td_on_event` call.
    pub event_fuel: u64,
}

fn engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    Engine::new(&config)
}

impl WasmModule {
    /// Instantiate `bytes`. `seed` feeds the module's `td::random`.
    pub fn load(name: &str, bytes: &[u8], seed: u64) -> Result<WasmModule, String> {
        let engine = engine();
        let module =
            Module::new(&engine, bytes).map_err(|e| format!("{name}: invalid wasm: {e}"))?;
        let host = Host {
            name: name.to_string(),
            rng: Rng::new(seed),
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).build(),
            logs: Vec::new(),
        };
        let mut store = Store::new(&engine, host);
        store.limiter(|h| &mut h.limits);
        store.set_fuel(SETUP_FUEL).map_err(|e| e.to_string())?;

        let mut linker = Linker::<Host>::new(&engine);
        linker
            .func_wrap(
                "td",
                "log",
                |caller: Caller<'_, Host>, ptr: u32, len: u32| {
                    let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory())
                    else {
                        return;
                    };
                    let data = memory.data(&caller);
                    let (start, end) = (ptr as usize, ptr as usize + len.min(4096) as usize);
                    if let Some(bytes) = data.get(start..end) {
                        let line = String::from_utf8_lossy(bytes).into_owned();
                        let mut caller = caller;
                        let host = caller.data_mut();
                        if host.logs.len() < 1000 {
                            host.logs.push(line);
                        }
                    }
                },
            )
            .map_err(|e| e.to_string())?;
        linker
            .func_wrap("td", "random", |mut caller: Caller<'_, Host>| -> i64 {
                caller.data_mut().rng.next_u64() as i64
            })
            .map_err(|e| e.to_string())?;

        let instance: Instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("{name}: {e}"))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| format!("{name}: no `memory` export"))?;
        let version: TypedFunc<(), u32> = instance
            .get_typed_func(&store, "td_abi_version")
            .map_err(|_| format!("{name}: missing `td_abi_version`"))?;
        let v = version
            .call(&mut store, ())
            .map_err(|e| format!("{name}: {e}"))?;
        if v != ABI_VERSION {
            return Err(format!(
                "{name}: ABI version {v}, engine speaks {ABI_VERSION}"
            ));
        }
        let alloc = instance
            .get_typed_func(&store, "td_alloc")
            .map_err(|_| format!("{name}: missing `td_alloc`"))?;
        let free = instance
            .get_typed_func(&store, "td_free")
            .map_err(|_| format!("{name}: missing `td_free`"))?;
        let init = instance.get_typed_func(&store, "td_init").ok();
        let on_event = instance.get_typed_func(&store, "td_on_event").ok();
        let generate = instance.get_typed_func(&store, "td_generate_map").ok();
        Ok(WasmModule {
            name: name.to_string(),
            store,
            memory,
            alloc,
            free,
            init,
            on_event,
            generate,
            event_fuel: EVENT_FUEL,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn has_rules(&self) -> bool {
        self.on_event.is_some()
    }

    pub fn has_generator(&self) -> bool {
        self.generate.is_some()
    }

    /// Lines the module logged since the last call.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().logs)
    }

    fn call(
        &mut self,
        f: TypedFunc<(u32, u32), i64>,
        input: &[u8],
        fuel: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let name = &self.store.data().name.clone();
        self.store.set_fuel(fuel).map_err(|e| e.to_string())?;
        let len = u32::try_from(input.len()).map_err(|_| "input too large")?;
        let ptr = self
            .alloc
            .call(&mut self.store, len)
            .map_err(|e| format!("{name}: td_alloc: {e}"))?;
        self.memory
            .write(&mut self.store, ptr as usize, input)
            .map_err(|_| format!("{name}: td_alloc returned a bad pointer"))?;
        let ret = f.call(&mut self.store, (ptr, len)).map_err(|e| {
            if self.store.get_fuel().unwrap_or(1) == 0 {
                format!("{name}: out of fuel")
            } else {
                format!("{name}: {e}")
            }
        })?;
        if ret < 0 {
            return Err(format!("{name}: returned error {ret}"));
        }
        if ret == 0 {
            return Ok(None);
        }
        let (out_ptr, out_len) = ((ret as u64 >> 32) as u32, ret as u64 as u32);
        let mut out = vec![0u8; out_len as usize];
        self.memory
            .read(&self.store, out_ptr as usize, &mut out)
            .map_err(|_| format!("{name}: output out of bounds"))?;
        self.free
            .call(&mut self.store, (out_ptr, out_len))
            .map_err(|e| format!("{name}: td_free: {e}"))?;
        Ok(Some(out))
    }

    pub fn init(&mut self, init: &ModuleInit) -> Result<(), String> {
        let Some(f) = self.init else { return Ok(()) };
        let input = encode(init)?;
        self.call(f, &input, SETUP_FUEL).map(|_| ())
    }

    pub fn generate_map(&mut self, params: &Value) -> Result<Map, String> {
        let f = self
            .generate
            .ok_or_else(|| format!("{}: not a map generator", self.name))?;
        let input = encode(params)?;
        let out = self
            .call(f, &input, SETUP_FUEL)?
            .ok_or_else(|| format!("{}: generator returned nothing", self.name))?;
        decode(&out).map_err(|e| format!("{}: bad map: {e}", self.name))
    }

    pub fn handle(&mut self, event: &Event) -> Result<Vec<Command>, String> {
        let Some(f) = self.on_event else {
            return Ok(Vec::new());
        };
        let input = encode(event)?;
        match self.call(f, &input, self.event_fuel)? {
            None => Ok(Vec::new()),
            Some(out) => decode(&out).map_err(|e| format!("{}: bad commands: {e}", self.name)),
        }
    }
}

impl Rule for WasmModule {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_event(&mut self, event: &Event, out: &mut Vec<Command>) -> Result<(), String> {
        out.extend(self.handle(event)?);
        Ok(())
    }
}
