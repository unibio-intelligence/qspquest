//! No host imports: UTF-8 JSON in and out, backed by the same engine.
#[path = "../../mcp/src/core/compute.rs"]
mod compute;
use serde_json::{json, Value};
#[no_mangle]
pub extern "C" fn qspquest_alloc(len: u32) -> u32 {
    let mut b = vec![0u8; len as usize].into_boxed_slice();
    let p = b.as_mut_ptr() as u32;
    std::mem::forget(b);
    p
}
#[no_mangle]
pub unsafe extern "C" fn qspquest_free(ptr: u32, len: u32) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        ptr as *mut u8,
        len as usize,
    )));
}
#[no_mangle]
pub unsafe extern "C" fn qspquest_execute(ptr: u32, len: u32) -> u64 {
    let bytes = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    let result = (|| {
        if len > 2_000_000 {
            return Err(json!({"code":"input_limit","message":"WASM input exceeds 2 MB"}));
        }
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| json!({"code":"invalid_input","message":e.to_string()}))?;
        if let Some(d) = v.get("library_document") {
            let d = serde_json::from_value(d.clone())
                .map_err(|e| json!({"code":"invalid_library_document","message":e.to_string()}))?;
            return compute::execute_document(
                &d,
                v["operation"]
                    .as_str()
                    .ok_or_else(|| json!({"message":"operation required"}))?,
                v["request"].clone(),
            );
        }
        compute::execute(
            v["source"]
                .as_str()
                .ok_or_else(|| json!({"message":"source required"}))?,
            v["operation"]
                .as_str()
                .ok_or_else(|| json!({"message":"operation required"}))?,
            v["request"].clone(),
        )
    })();
    let output = match result {
        Ok(v) => json!({"ok":v}),
        Err(e) => json!({"error":e}),
    }
    .to_string();
    let mut b = output.into_bytes().into_boxed_slice();
    let ptr = b.as_mut_ptr() as u32;
    let len = b.len() as u32;
    std::mem::forget(b);
    ((len as u64) << 32) | (ptr as u64)
}
