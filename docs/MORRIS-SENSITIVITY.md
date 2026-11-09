# Morris elementary-effect screening

`CompiledDocument::morris` runs Morris trajectories [1] using
[parameter scans](PARAMETER-SCANS.md). Inputs give parameter ranges as
quantities, ordered trajectories of point IDs, an even grid level count, and one
output column/row. Each trajectory changes each selected parameter once, one
parameter per step. Every point must belong to exactly one trajectory and must
set exactly the selected parameters. Other model parameters and the regimen
stay fixed.

The normalized grid has p even levels (4–100), spacing 1/(p−1), and step magnitude
Δ=p/[2(p−1)]. Steps may go in either direction. Parameters are scaled to [0, 1]
using the supplied ranges, after unit conversion. Grid checks allow 1e-8
error in grid-index coordinates and 1e-12 at range endpoints to absorb
roundoff. Simulations use the supplied parameter values unchanged.

For each step, the elementary effect (EE) is EE=(output_after−output_before)/signed_normalized_step. For each
parameter, the result includes all EEs, their signed mean `mu`, the mean of their
absolute values `mu_star` [2], and sample standard deviation `sigma` (denominator
n−1). Because inputs are normalized, these quantities have the selected output's
units. They depend on the specified parameter ranges and output selection.
The definitions follow the unscaled, ungrouped Morris metrics documented by
[SALib](https://salib.readthedocs.io/en/main/api.html) [3]. No bootstrap confidence
intervals or standard-deviation scaling are computed.

The engine requires at least two trajectories, 1–11 selected parameters and no
more than 25 scan points. The scan's budget rules still apply. Designs are
checked before simulation. All runs must produce the same time/side grid so an
output row means the same thing throughout; designs whose parameters change the
event grid are rejected. Nonfinite effects or summary overflow return
structured errors. The result includes the full scan results and a hash of the
request.

Tests compare effects and all three summaries with independently evaluated
one-compartment PK formulas, including forward/reverse steps and mixed units.

Trajectories can be supplied explicitly or generated (below). Trajectory
optimization and bootstrap intervals are not available. Two trajectories are
sufficient to compute the statistics, but not to sample a nonlinear model
adequately; the design should cover the parameter range of interest. A large sigma can
reflect nonlinearity or interactions; the Morris method does not distinguish between them.

## Trajectory generation

`qspquest_core::morris::generate_morris_design` accepts the shared run, quantity
ranges, trajectory count, even grid level count, a u32 seed, aggregate budgets
and output selection. It returns a complete `MorrisRequest` plus the seed,
generator name and design-request hash. Generation does not simulate; parameter
names, domains and run validity are checked when the analysis runs.

The generator implements ungrouped, unoptimized Morris sampling: independent
base grid indices and direction choices, with a Fisher–Yates parameter
permutation per trajectory [4, 5]. Its grid geometry follows the standard design
shown in [SALib's sampling implementation](https://salib.readthedocs.io/en/latest/_modules/SALib/sample/morris/morris.html) [3].
The same seed does not give the same design as NumPy/SALib.

Generator `qspquest_morris_sha256_counter_v1` hashes the UTF-8 domain string
`qspquest.morris.sha256-counter.v1`, followed by the seed as four little-endian
bytes and a zero-based counter as eight little-endian bytes. Each digest's
first eight bytes form a little-endian u64. Bounded integer draws reject values
below `2^64 mod n` before reducing modulo n, with at most 256 attempts per draw.
This makes the stream explicit and reproducible without a new random number generator (RNG) dependency.
For each trajectory, permutation draws precede per-parameter base/direction
pairs. Direction draw zero means increasing. Every generated point has a unique
trajectory/step ID; coordinates may repeat across independently drawn paths.

Requests exceeding 25 runs are rejected before allocation/generation. Ranges
must be finite, increasing and unit-compatible. Quantities use the lower bound's
unit; exact grid endpoints use the converted bounds directly. The design hash
covers all generator inputs and the algorithm name.

Tests check a fixed-seed grid calculated independently, exact repeatability and
16 seed/grid combinations run end to end.

## Versioned CLI execution

`qspquest morris MODEL REQUEST.json` accepts model JSON or `.qsp` text and a
`qspquest.morris/v0.1` envelope. `problem.kind` is `generated` or `explicit`;
`problem.request` is respectively a design-generation request or a complete
explicit analysis request. An example is
`conformance/requests/synthetic-morris.json`.

`CompiledDocument::execute_morris` runs the same envelope in Rust. Both paths
return `qspquest.morris-result/v0.1`, containing the analysis result and an
optional `design` record. Generated designs record the generator name, seed and
design-request hash; for explicit designs these are null. An explicit design
and its generated equivalent give identical effects but different analysis
hashes.

Generated request/result schemas are in `spec/schema`. Unknown versions or
fields are rejected. Limits, units and run validity are checked at run time.
The CLI keeps its file-size limit and structured stderr errors.

## Python and browser execution

Python `CompiledModel.morris(request)` returns a dict and `morris_json(request)`
returns JSON. They accept the same versioned envelope, release the global interpreter lock (GIL), and allow
a compiled model to be shared across analyses.

WebAssembly (WASM) `CompiledSimulation.morris(request_json)` and conformance-worker
`kind: "morris"` execute the same request and reproduce seeded designs and
request hashes.

## References

1. Morris MD. Factorial sampling plans for preliminary computational
   experiments. *Technometrics*. 1991;33(2):161–174.
   [doi:10.1080/00401706.1991.10484804](https://doi.org/10.1080/00401706.1991.10484804)
2. Campolongo F, Cariboni J, Saltelli A. An effective screening design for
   sensitivity analysis of large models. *Environ Model Softw*.
   2007;22(10):1509–1518.
   [doi:10.1016/j.envsoft.2006.10.004](https://doi.org/10.1016/j.envsoft.2006.10.004)
3. Herman J, Usher W. SALib: an open-source Python library for sensitivity
   analysis. *J Open Source Softw*. 2017;2(9):97.
   [doi:10.21105/joss.00097](https://doi.org/10.21105/joss.00097)
4. Fisher RA, Yates F. *Statistical Tables for Biological, Agricultural and
   Medical Research*. London: Oliver & Boyd; 1938.
5. Durstenfeld R. Algorithm 235: random permutation. *Commun ACM*.
   1964;7(7):420.
   [doi:10.1145/364520.364540](https://doi.org/10.1145/364520.364540)
