# Prepare and fit explicit population records

This tutorial shows how dataset records (NONMEM-style rows) become a population
fitting request, what an input error looks like, and how to run one small
synthetic FOCEI fit. You need Python 3.10 or later, a Rust toolchain and the
QSPQuest source distribution. Run these commands from the distribution root:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_population_records.py target/release/qspquest-mcp \
  --out population-record-results
```

The [complete client](../examples/mcp_population_records.py) uses the shipped
[synthetic input](../../bindings/mcp/assets/fits/population.json) and the
[standard-library transport](../examples/stdio_rpc.py). Choose a new output
directory with an existing parent. The client first sends a deliberately broken
endpoint table to `plan_run` to show the error, then previews the real records
and calls `fit_population` with that plan's hash. It saves the inputs, the error,
the plan, the full result and checks. Keep its `artifacts/` directory private.

Expected checks report four subjects, 24 included observations, an error
pointing at `mapping.endpoints`, matching plan hashes, convergence, and
clearance within 0.06 L/h of the true 0.6 L/h. A failed fit exits with a nonzero
status and keeps its diagnostics. This is a teaching example, not evidence
of clinical validity.

The fixture declares time in hours, dosing amounts in mg, and the concentration
observable `cp` in mg/L. The explicit mapping binds CMT `1` to native dosing
state `central` and CMT `2` to `cp`. Each observation declares its additive and
proportional Gaussian error. The population template supplies sensitivities,
bounds, fixed Omega, method limits and seed. Units and model structure are never
guessed from column names.

Record admission supports EVID 0/1, MDV 0/1 and SS 0. A dose needs positive AMT;
positive RATE needs an explicit `mapping.rate_unit`. ADDL is an integer from 0
through 10000 and needs a positive II when nonzero. Observation records must not
carry nonzero AMT, RATE, ADDL or II. Included observations need finite DV. An
excluded observation is ignored. Text and integer subject IDs are treated as
different. Row order is kept and recorded in `row_mapping`; rows are never
sorted. Hosted use allows at most 16 subjects and 256 included observations;
local use allows 128 subjects and 10000. Exceeding a limit gives a `budget`
error; subjects and rows are never dropped to fit.

The mapping object accepts only `time_unit`, `amount_unit`, `rate_unit`,
`observation_side`, `compartments` and `endpoints`. Each endpoint accepts only
`output`, `value_unit` and the native `error` object. Observation side defaults
to `post`; an explicit value must be `pre` or `post`. The method defaults to
`focei`; `laplace` and `saem` can be chosen explicitly. A missing or null optional
value uses its default. Misspelled fields, wrong shapes and unsupported values
are rejected with an error. Extra record columns are allowed and ignored.

Read `structuredContent.error.input_diagnostic.field`, `supplied_value` and
`expected` to find the problem. Long values are shortened. Correct the named
field, call `plan_run` again, and submit with the new `expected_plan_hash`.

The engine still checks units, output names, random effects and limits when
it fits, and FOCEI, Laplace and SAEM each have their own restrictions. Check the
objective kind, bounds, estimates, subject diagnostics and any uncertainty. Do
not compare a complete-data surrogate with a marginal likelihood, or report
non-converged estimates as final. You can also skip record mapping and pass a
complete `fit_request` directly.

The mapping compatibility fixtures can be checked separately:

```sh
cargo test --locked -p qspquest-mcp core::population_records::tests
```

These tests cover record mapping and input errors only; they do not run a fit.
