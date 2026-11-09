# QSPQuest modeling

Model compiler, simulation, sensitivity and fitting engine for QSPQuest.

Add `qspquest-modeling = "=0.1.1"` to your Cargo dependencies. This crate uses
[Diffsol](https://crates.io/crates/diffsol) 0.18 from crates.io.

QSPQuest's own CLI, Python, R and MCP builds use a newer upstream Diffsol
revision with BDF fixes, and its validation applies to those builds. With
Diffsol 0.18.0, a solve has no Diffsol step limit (the callback budget still
bounds it), and some population fits may not certify a covariance. Each result
records the Diffsol source it ran with in `backend_version`.
