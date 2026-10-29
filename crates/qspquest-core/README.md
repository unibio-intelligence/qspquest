# QSPQuest core

Solver-independent scientific request, dosing-event and result types.

```rust
use qspquest_core::Event;

let dose = Event::Bolus { time: 0.0, target: 0, amount: 6.0 };
assert_eq!(dose.time(), 0.0);
```

Add `qspquest-core = "=0.1.0-rc.1"` to your Cargo dependencies.
Enable `schema` to generate JSON schemas. For model execution, use the QSPQuest
Python package or the complete source checkout.
