//! Browser bindings and the isolated hosted engine ABI.
#[cfg(not(feature = "sandbox-abi"))]
mod browser;
#[cfg(not(feature = "sandbox-abi"))]
pub use browser::*;
#[cfg(feature = "sandbox-abi")]
mod sandbox;
