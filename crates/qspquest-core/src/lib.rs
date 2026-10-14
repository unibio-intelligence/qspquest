//! Solver-independent scientific request and result types.
pub mod dose_history;
pub mod expression;
pub mod fit;
pub mod identity;
pub mod initial_values;
pub mod model;
pub mod morris;
pub mod planning;
pub mod regimen;
pub mod run;
pub mod scan;
#[cfg(feature = "schema")]
pub mod schema;
pub mod units;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveInputs {
    Continue,
    StopTarget,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Reset {
        time: f64,
        target: usize,
        value: f64,
        order: u32,
        active_inputs: ActiveInputs,
    },
    Bolus {
        time: f64,
        target: usize,
        amount: f64,
    },
    Infusion {
        time: f64,
        target: usize,
        amount: f64,
        duration: f64,
    },
}
impl Event {
    pub fn time(&self) -> f64 {
        match self {
            Self::Bolus { time, .. } | Self::Infusion { time, .. } | Self::Reset { time, .. } => {
                *time
            }
        }
    }
    pub fn target(&self) -> usize {
        match self {
            Self::Bolus { target, .. }
            | Self::Infusion { target, .. }
            | Self::Reset { target, .. } => *target,
        }
    }
    pub fn amount(&self) -> Option<f64> {
        match self {
            Self::Bolus { amount, .. } | Self::Infusion { amount, .. } => Some(*amount),
            Self::Reset { .. } => None,
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protocol {
    #[serde(default)]
    pub start: f64,
    pub end: f64,
    pub samples: Vec<f64>,
    pub events: Vec<Event>,
    pub rtol: f64,
    pub atol: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub time: f64,
    pub side: String,
    pub values: Vec<f64>,
}

/// Stable failure categories. No partial scientific output accompanies an error.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    Unit,
    UnknownUnit,
    Type,
    UnknownSymbol,
    ExpressionLimit,
    Unsupported,
    Domain,
    Invariant,
    Solver,
    WorkBudget,
    OutputBudget,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    pub time: Option<f64>,
    pub expression: Option<String>,
}
impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            time: None,
            expression: None,
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }
}
/// Work counts RHS and Jacobian calls, including solver setup, or bounded
/// analytic interval propagations when an analytic model opts into that path.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub solver_callbacks: u64,
    pub output_values: usize,
    pub events: usize,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            solver_callbacks: 1_000_000,
            output_values: 1_000_000,
            events: 10_000,
        }
    }
}
