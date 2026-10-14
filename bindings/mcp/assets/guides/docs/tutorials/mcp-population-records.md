# Prepare and fit explicit population records

Use this tutorial to inspect how source records become a native population
request, obtain an actionable admission error, and run one bounded synthetic
FOCEI example. It requires Python 3.10 or later, a Rust toolchain, and the
QSPQuest source distribution. No downloaded model, platform account or patient
data is required. Run these commands from the distribution root:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_population_records.py target/release/qspquest-mcp \
  --out population-record-results
```

The [complete client](qspquest://documentation/files/docs/examples/mcp_population_records.py) uses the shipped
[synthetic input](qspquest://documentation/files/bindings/mcp/assets/fits/population.json) and the
[standard-library transport](qspquest://documentation/files/docs/examples/stdio_rpc.py). Choose a new output
directory with an existing parent. The client never replaces existing results.
It first checks a malformed endpoint table with `plan_run`, prepares all source
rows without fitting, and calls `fit_population` with that plan's exact hash.
The script retains inputs, refusal, complete plan, full result and checks. Its
`artifacts/` directory retains the server's complete results; keep it private.

Expected checks report four subjects, 24 included observations, a located
`mapping.endpoints` refusal, matching row provenance and plan hashes,
convergence, and clearance recovery within the fixture's declared 0.06 L/h
tolerance around 0.6 L/h. A failed fit exits with a nonzero status and retains
its diagnostic candidates. One successful teaching example does not establish
population bias, uncertainty coverage, broad design performance or clinical
validity. `scientific_qualification` remains `not_assessed`.

The fixture declares time in hours, dosing amounts in mg, and the concentration
observable `cp` in mg/L. The explicit mapping binds CMT `1` to native dosing
state `central` and CMT `2` to `cp`. Each observation declares its additive and
proportional Gaussian error. The population template supplies sensitivities,
bounds, fixed Omega, method budgets and seed. These quantities remain distinct;
the adapter does not infer units or statistical structures from column names.

Record admission supports EVID 0/1, MDV 0/1 and SS 0. A dose needs positive AMT;
positive RATE needs an explicit `mapping.rate_unit`. ADDL is an integer from 0
through 10000 and needs a positive II when nonzero. Observation records must not
carry nonzero AMT, RATE, ADDL or II. Included observations need finite DV. An
excluded observation is not imputed and its DV is not used. Text and signed
integer subject IDs remain distinct. Source order is retained in `row_mapping`;
the mapper does not silently sort a scientific protocol.
Hosted record admission allows at most 16 subjects and 256 included
observations; local admission allows 128 subjects and 10000 included
observations. These are shared with native request admission and checked before
expanded subject objectives are materialized. Exceeding a limit produces a
located `budget` refusal. The mapper never drops subjects or downsamples rows.

The mapping object accepts only `time_unit`, `amount_unit`, `rate_unit`,
`observation_side`, `compartments` and `endpoints`. Each endpoint accepts only
`output`, `value_unit` and the native `error` object. Observation side defaults
to `post`; an explicit value must be `pre` or `post`. The method defaults to
`focei`; choose `laplace` or `saem` explicitly only with a supported native
contract. An absent or null optional value uses its default. Wrong container
shapes, misspelled mapping fields, unsupported side values and malformed native
templates produce admission errors. Extra descriptive record columns are
allowed and never become numerical inputs or authorization instructions.

Read `structuredContent.error.input_diagnostic.field`, `supplied_value` and
`expected` to locate a refusal. Long field names or values are shortened with
an explicit digest of their complete value. The included request example is a
shape illustration; it never replaces your data, doses, errors or template.
Correct the named field, inspect `plan_run` again, and submit the same intended
arguments with the new `expected_plan_hash`.

Units, output names, random-effect dimensions, likelihood support, budgets and
convergence remain subject to native engine preparation and execution. Native
FOCEI/Laplace/SAEM contracts have their own restrictions. Inspect the retained
objective kind, bounds, fixed-effect estimates, subject diagnostics and any
available uncertainty. Do not compare a complete-data surrogate with a marginal
likelihood or report nonconverged candidates as final estimates. A complete
`fit_request` bypasses record mapping and still receives native admission checks.

The mapping compatibility fixtures can be checked separately:

```sh
cargo test --locked -p qspquest-mcp core::population_records::tests
```

These checks cover the own golden request and source-row mappings, malformed
containers, located numeric errors, excluded observations, explicit infusion
and repeat units, and pre/post conventions. They test admission and mapping;
they perform no population estimation or statistical qualification.
