# Gaussian likelihood for narrow fitting

`CompiledSensitivityDocument::gaussian_objective` evaluates a negative log
likelihood and its gradient in the fitted parameters' units.
The individual and pooled fitters described below use this objective.
Set `simulation.run.solver` to `diffsol_bdf` (the default in helpers).
Other solvers are not supported for sensitivity-based fitting.

The request contains a sensitivity run, an observation table and optional
independent Gaussian priors. Each observation names an output and a zero-based
row in the run's ordered result grid, plus a measured quantity and error model.
Duplicate rows represent independent replicate measurements. Omit missing
measurements. A row with `censoring: "left"` supplies its lower quantification
limit as `value`; its likelihood is the probability of an observation below
that limit. Correlated residuals are not supported.

The error coefficients are known constants for this operation:

- `additive_sd` is a quantity in the observable's units.
- `proportional_sd` is dimensionless.
- Both are nonnegative and cannot both be zero.
- Optional `model: "lognormal"` uses zero additive SD and treats
  `proportional_sd` as the standard deviation on the log-observation scale.

For prediction μ, variance is `additive_sd² + (proportional_sd · μ)²`.
The score includes the normal density normalization and the variance derivative,
not only a weighted residual term. Pure proportional error at zero prediction
is a domain error. Data and additive standard deviations are converted to the
model's output unit, and densities are reported in that unit.
For the lognormal model, both prediction and observed value (or censoring
limit) must be positive. An uncensored observation uses the lognormal density,
including its `log(value)` Jacobian; a censored observation uses the normal
CDF of `log(limit) - log(prediction)` divided by the log-scale SD.

Optional priors name fitted parameters and give a mean and positive standard
deviation. Priors are normal densities on the untransformed parameter values in
the model's units, with no Jacobian term. No priors means a maximum-likelihood objective. Residual coefficients
are not estimated by this request.

The result separates negative log likelihood, negative log prior and their
sum, and returns named gradient values with inverse-parameter units.
Invalid units, nonfinite/zero variance, unknown outputs, out-of-range rows,
duplicate prior names, overflow and exhausted storage budgets fail without a
partial objective.

Tests check additive, proportional, combined, and lognormal errors, including
left censoring, against independently written one-compartment likelihoods and
finite differences.

## Native bounded individual fitting

`CompiledSensitivityDocument::fit_gaussian` minimizes the objective with
projected inverse BFGS and an Armijo line search. Each fitted parameter needs
finite lower and upper bounds, in the same order as the parameters. The bounds
must contain the initial value and lie inside the model's own bounds. Fixed
parameters cannot be fitted.

The optimizer rescales each parameter's bounds to [0, 1]; it ignores the
model's log/logit/probit transforms. The gradient tolerance applies to the
largest projected gradient component on this rescaled scale, so it depends on
how wide the bounds are.

The result distinguishes convergence, evaluation limit, iteration limit and
line-search failure, and keeps the best parameters found and their score.
A nonconverged result is not a successful fit. Domain, solver and invariant
failures during trial steps cause backtracking; malformed requests and exhausted
simulation budgets return errors. Failed trials count toward the evaluation
limit. Each evaluation uses the requested simulation budgets; total work is
bounded by those budgets times the evaluation limit (at most 10,000). Iterations
are separately bounded by 10,000. Convergence does not guarantee a global
optimum or an identifiable model. Integration tolerances must be tight enough
for the requested gradient accuracy; line-search failure near an optimum can
indicate integration noise.

Tests recover clearance and volume from analytic observations, including
repeated boluses and a reset, a boundary optimum and a normal-prior MAP fit,
using tight integration tolerances (rtol 1e-12). Fitting on transformed scales
and uncertainty estimates are not yet available.

## Pooled fitting

`fit_pooled_gaussian` accepts a primary `GaussianFitRequest` and up to 1,023
additional subject objectives. All subjects use the same compiled model and
share the fitted parameters. Each subject keeps its own regimen, covariates,
initial-state overrides, other parameter values, observations, residual
coefficients and simulation budgets. Initial values of the fitted parameters
must agree across subjects after unit conversion; conflicting values are
rejected. The primary request supplies the shared bounds and optimizer controls.

Declare priors only in the primary objective, so the prior counts exactly once;
priors on additional subjects are rejected. Likelihoods and gradients sum over
subjects in request order. Each pooled evaluation counts once toward the
evaluation limit, and its maximum solver work is the sum of the subject budgets.
Subjects run one after another; separate fits can run in parallel.

The result holds the total likelihood, one prior, combined gradient and total
observation count. If any subject fails, no pooled score is returned. Tests
recover clearance and volume from two different dose regimens. This is pooled
estimation with shared parameters; random effects and population NLME
estimation are not part of this API.

## Versioned file interface

The CLI accepts `qspquest fit MODEL REQUEST.json`, where MODEL is model JSON
or `.qsp` text. JSON models permit 4,000,000 bytes; text and requests permit
1,000,000 bytes. The request schema
is `qspquest.fit/v0.1`. Its `problem` contains `kind` (`individual` or `pooled`)
and `request` (the corresponding typed request above). A complete executable
example is `conformance/requests/synthetic-fit.json`.

Results use `qspquest.fit-result/v0.1` with a `fit` object. Inspect `fit.status`:
only `converged` meets the requested optimizer criterion. Exit code zero means
the operation returned a result, including a nonconverged result; it does not
assert successful estimation. Invalid input and execution failures print a
structured error to stderr and exit nonzero. Input files are never modified.
Generated request/result schemas are in `spec/schema`.

The Python `CompiledSensitivities` class exposes `fit(request)` (dict result) and
`fit_json(request)` (JSON text). Both accept this same versioned envelope. Compile
with the fitted parameters in the request's order, then reuse the instance
across fits or threads. Fitting releases the GIL; requests keep the
one-million-byte limit. Failures raise `QSPQuestError`. Nonconverged fits
return an explicit status.

The WASM `CompiledSensitivities.fit(request_json)` method accepts the same
versioned envelope. The conformance worker handles `kind: "fit"` and caches
only the compiled model and parameter order. Cancelling terminates the worker;
the next request starts a new one. The worker is currently built with the
`m0-conformance` feature.

Chrome 154 and Firefox 154 are tested with three individual fits, one pooled
fit, structured failures, nonconvergence, cancellation and recovery.
Parameters, objectives and gradients agree with native results within 1e-5
(maximum observed absolute difference approximately 1.94e-12). Safari is not
yet tested for fitting.
