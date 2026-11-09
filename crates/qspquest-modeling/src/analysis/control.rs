//! Progress, cooperative cancellation and checkpoint hooks for long analyses.
//!
//! Hooks run only on the thread that called the analysis. Worker threads read
//! the cancellation flag between native calls and never call the control.
use super::common::{AnalysisError, Result};
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Shared cancellation request, read between native calls.
pub type CancelFlag = Arc<AtomicBool>;

pub trait Control {
    /// Called once with the operation name and its request identity. Returns
    /// a saved state when resuming; the binding verifies that the saved
    /// context equals `context` before returning it.
    fn start(&mut self, operation: &str, context: &Value) -> Result<Option<Value>>;
    /// Whether the next boundary should receive a checkpoint state.
    fn wants_state(&self, completed: usize, final_boundary: bool) -> bool;
    /// Report one completed work unit. `progress` has the operation,
    /// completed and total units, and simulations used and allowed.
    fn boundary(&mut self, progress: &Value, state: Option<Value>) -> Result<()>;
    /// Persist the completed prefix after a drained cancellation or budget stop.
    fn retain(&mut self, state: Value) -> Result<()>;
    fn cancel_flag(&self) -> Option<CancelFlag>;
}

/// Control for callers that need neither progress nor checkpoints.
pub struct NoControl;

impl Control for NoControl {
    fn start(&mut self, _: &str, _: &Value) -> Result<Option<Value>> {
        Ok(None)
    }
    fn wants_state(&self, _: usize, _: bool) -> bool {
        false
    }
    fn boundary(&mut self, _: &Value, _: Option<Value>) -> Result<()> {
        Ok(())
    }
    fn retain(&mut self, _: Value) -> Result<()> {
        Ok(())
    }
    fn cancel_flag(&self) -> Option<CancelFlag> {
        None
    }
}

/// Emit one boundary, building the state only when it is wanted.
pub fn boundary(
    control: &mut dyn Control,
    operation: &str,
    completed: usize,
    total: usize,
    used: usize,
    maximum: usize,
    final_boundary: bool,
    state: impl FnOnce() -> Value,
) -> Result<()> {
    let progress = serde_json::json!({
        "operation": operation,
        "completed": completed,
        "total": total,
        "simulations": used,
        "max_simulations": maximum,
    });
    let state = control.wants_state(completed, final_boundary).then(state);
    control.boundary(&progress, state)?;
    if let Some(flag) = control.cancel_flag() {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AnalysisError::new(
                "cancelled",
                "Analysis cancelled. Resume the last complete checkpoint with a new control; unfinished work is discarded.",
            ));
        }
    }
    Ok(())
}
