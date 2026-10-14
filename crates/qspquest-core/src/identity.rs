//! Canonical JSON hashes for scientific content, independent of the host runtime.
use crate::{Error, ErrorCode};
use serde_json::Value;
use sha2::{Digest, Sha256};
pub const SPEC_VERSION: &str = "qspquest.spec/0.1";
/// RFC 8785 JCS, then SHA-256. Large integer literals must be represented as
/// strings; f64 scientific values use the JCS binary64 number representation.
pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>, Error> {
    fn check(value: &Value, depth: usize) -> Result<(), Error> {
        if depth > 256 {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "canonical JSON depth limit exceeded",
            ));
        }
        match value {
            Value::Number(n)
                if n.as_u64().is_some_and(|v| v > 9_007_199_254_740_991)
                    || n.as_i64().is_some_and(|v| v < -9_007_199_254_740_991) =>
            {
                return Err(Error::new(
                    ErrorCode::InvalidInput,
                    "integer exceeds exact interoperable JSON range",
                ))
            }
            Value::Array(items) => {
                for item in items {
                    check(item, depth + 1)?;
                }
            }
            Value::Object(items) => {
                for item in items.values() {
                    check(item, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    check(value, 0)?;
    serde_jcs::to_vec(value)
        .map_err(|e| Error::new(ErrorCode::InvalidInput, format!("canonical JSON: {e}")))
}
pub fn canonical_hash(value: &Value) -> Result<String, Error> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(canonical_bytes(value)?)
    ))
}
