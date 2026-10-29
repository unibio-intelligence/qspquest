//! Closed expression nodes map to the corresponding Ubi ModelIR/v2 fields.
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Number(f64),
    Boolean(bool),
}
macro_rules! operators { ($name:ident { $($op:ident),* }) => {
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all="snake_case")]
    pub enum $name { $($op),* }
}; }
operators!(Unary {
    Positive,
    Negative,
    Not
});
operators!(Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Modulo
});
operators!(Compare {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne
});
operators!(Boolean { And, Or });
operators!(Function {
    Exp,
    Log,
    Log10,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Tanh,
    Abs,
    Min,
    Max,
    Floor,
    Ceil,
    Hill,
    InverseGaussianDensity,
    Ifelse
});
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    Literal {
        value: Literal,
    },
    /// A finite numeric constant with an explicit dimensional unit.
    Quantity {
        quantity: crate::model::Quantity,
    },
    Symbol {
        name: String,
    },
    Unary {
        operator: Unary,
        arguments: Box<[Expr; 1]>,
    },
    Binary {
        operator: Binary,
        arguments: Box<[Expr; 2]>,
    },
    Compare {
        operator: Compare,
        arguments: Box<[Expr; 2]>,
    },
    Boolean {
        operator: Boolean,
        arguments: Vec<Expr>,
    },
    Call {
        name: Function,
        arguments: Vec<Expr>,
    },
    Conditional {
        arguments: Box<[Expr; 3]>,
    },
}
impl Expr {
    pub fn number(value: f64) -> Self {
        Self::Literal {
            value: Literal::Number(value),
        }
    }
    pub fn symbol(name: impl Into<String>) -> Self {
        Self::Symbol { name: name.into() }
    }
    pub fn binary(operator: Binary, a: Self, b: Self) -> Self {
        Self::Binary {
            operator,
            arguments: Box::new([a, b]),
        }
    }
    pub fn call(name: Function, arguments: Vec<Self>) -> Self {
        Self::Call { name, arguments }
    }
    pub fn conditional(condition: Self, yes: Self, no: Self) -> Self {
        Self::Conditional {
            arguments: Box::new([condition, yes, no]),
        }
    }
    pub fn children(&self) -> &[Expr] {
        match self {
            Self::Literal { .. } | Self::Quantity { .. } | Self::Symbol { .. } => &[],
            Self::Unary { arguments, .. } => arguments.as_slice(),
            Self::Binary { arguments, .. } | Self::Compare { arguments, .. } => {
                arguments.as_slice()
            }
            Self::Conditional { arguments } => arguments.as_slice(),
            Self::Boolean { arguments, .. } | Self::Call { arguments, .. } => arguments,
        }
    }
}
