use serde_json::{json, Value};
use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimits, StoreLimitsBuilder};
pub struct Sandbox {
    engine: Engine,
    module: Module,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed)
    }
}
impl Sandbox {
    pub fn embedded() -> Result<Self, String> {
        Self::new(include_bytes!(concat!(env!("OUT_DIR"), "/sandbox.wasm")))
    }
    pub fn new(bytes: &[u8]) -> Result<Self, String> {
        let mut c = Config::new();
        c.epoch_interruption(true).consume_fuel(true);
        let engine = Engine::new(&c).map_err(|e| e.to_string())?;
        let module = Module::new(&engine, bytes).map_err(|e| e.to_string())?;
        if module.imports().next().is_some() {
            return Err("Sandbox module must have no host imports".into());
        }
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let e = engine.clone();
        std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(10));
                e.increment_epoch();
            }
        });
        Ok(Self {
            engine,
            module,
            stop,
        })
    }
    pub fn execute(&self, source: &str, operation: &str, request: Value) -> Result<Value, Value> {
        self.limited(source, operation, request, 45_000, 100_000_000_000)
    }
    pub fn execute_library(
        &self,
        document: Value,
        operation: &str,
        request: Value,
    ) -> Result<Value, Value> {
        self.envelope(
            json!({"library_document":document,"operation":operation,"request":request}),
            45_000,
            100_000_000_000,
        )
    }
    pub fn limited(
        &self,
        source: &str,
        operation: &str,
        request: Value,
        millis: u64,
        fuel: u64,
    ) -> Result<Value, Value> {
        self.envelope(
            json!({"source":source,"operation":operation,"request":request}),
            millis,
            fuel,
        )
    }
    /// Qualification-only fuel measurement; uses the same production limits.
    pub fn profile_library(
        &self,
        document: Value,
        operation: &str,
        request: Value,
    ) -> Result<(Value, u64), Value> {
        self.envelope_profile(
            json!({"library_document":document,"operation":operation,"request":request}),
            45_000,
            100_000_000_000,
        )
    }
    fn envelope(&self, input: Value, millis: u64, fuel: u64) -> Result<Value, Value> {
        self.envelope_profile(input, millis, fuel)
            .map(|(value, _)| value)
    }
    fn envelope_profile(
        &self,
        input: Value,
        millis: u64,
        fuel: u64,
    ) -> Result<(Value, u64), Value> {
        let execute = || -> Result<(Value, u64), String> {
            let limits = StoreLimitsBuilder::new()
                .memory_size(256 * 1024 * 1024)
                .instances(1)
                .memories(1)
                .tables(4)
                .trap_on_grow_failure(true)
                .build();
            let mut store: Store<StoreLimits> = Store::new(&self.engine, limits);
            store.limiter(|l| l);
            store.set_epoch_deadline(millis.div_ceil(10).max(1));
            store.set_fuel(fuel).map_err(|e| e.to_string())?;
            let instance =
                Instance::new(&mut store, &self.module, &[]).map_err(|e| e.to_string())?;
            let memory = instance
                .get_memory(&mut store, "memory")
                .ok_or("memory export missing")?;
            let alloc = instance
                .get_typed_func::<u32, u32>(&mut store, "qspquest_alloc")
                .map_err(|e| e.to_string())?;
            let call = instance
                .get_typed_func::<(u32, u32), u64>(&mut store, "qspquest_execute")
                .map_err(|e| e.to_string())?;
            let input = input.to_string();
            if input.len() > 2_000_000 {
                return Err("Sandbox input exceeds 2 MB".into());
            }
            let ptr = alloc
                .call(&mut store, input.len() as u32)
                .map_err(|e| e.to_string())?;
            memory
                .write(&mut store, ptr as usize, input.as_bytes())
                .map_err(|e| e.to_string())?;
            let packed = call
                .call(&mut store, (ptr, input.len() as u32))
                .map_err(|e| e.root_cause().to_string())?;
            let len = (packed >> 32) as usize;
            let ptr = packed as u32 as usize;
            if len > 8_000_000 {
                return Err("Sandbox result exceeds 8 MB".into());
            }
            let mut b = vec![0; len];
            memory
                .read(&store, ptr, &mut b)
                .map_err(|e| e.to_string())?;
            let value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
            Ok((
                value,
                fuel.saturating_sub(store.get_fuel().map_err(|e| e.to_string())?),
            ))
        };
        let (r, consumed)=execute().map_err(|message|json!({"error":{"code":"sandbox_limit","message":message,"suggestion":"The isolated instance exhausted fuel, memory or wall time, or trapped. Reduce work or run locally."}}))?;
        if let Some(e) = r.get("error") {
            Err(json!({"error":e}))
        } else {
            Ok((r["ok"].clone(), consumed))
        }
    }
}
