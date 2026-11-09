# Changelog

## 0.1.1

- QSPQuest has one SAEM engine. `saem` fit requests and the analysis
  `fit_population` and `bootstrap_population` run the same sampler and
  M-steps; the capability catalog lists one `saem` estimator.
- `saem` fit requests accept 1 to 8 effects, covariate effects on any random
  effect, `covariance: "diagonal"`, `sweeps`, and `proportional_error_fit`,
  alone or with `additive_error_fit` for a combined normal residual model.
  The marginal OFV refinement covers one effect with fixed residual error
  and up to four covariate effects, or with a fitted additive SD and no
  covariates. It also covers two effects with full Omega, no covariates and a
  fixed or fitted additive SD.
  Log-Wald uncertainty also needs fixed residual error and no covariates. Other
  fits report the complete-data surrogate. The one-effect marginal route
  admits up to 1,024 subjects, and other routes admit up to 128.
- With two or more effects the SAEM sampler adds an adaptive joint proposal,
  and proposal scales can shrink to one thousandth of the effect SD, so chains
  keep moving with precise data. Fit results change from earlier releases at
  the same seed.
- `estimated_residual_error` can report a fitted `proportional_sd`.
- The residual-error M-step of every SAEM fit solves its objective by closed
  form or projected Newton. The earlier solver could stall when one SD hardly
  affected the variance, which failed bootstrap replicates with a combined
  residual model. Analysis fits with fitted residual SDs change slightly.
- The projected-BFGS optimizer used by `fit_data` (gradient method),
  `profile_likelihood` and the analysis residual fits restarts its curvature
  estimate when a bound becomes active or inactive. It previously crept
  towards optima with an active bound and could stop at its iteration limit.
  Their results can change slightly.
- Python and R provide plotting and table helpers for simulation, fitting and
  predictive-check results, saved CSV tables, model comparisons and target attainment.
- Regimen figures accept single-percentile and two-percentile summaries, and BLQ
  predictive-check figures keep strata in separate panels.
- Single-dose and multiple-dose PK tutorials include runnable examples and figures.
- The all-in-one Docker image includes the CLI, Python, JupyterLab, R and the MCP server.
- Profile likelihood and the population residual-error fit reject a failed optimizer
  line search instead of reporting convergence.
- The capability catalog (`qspquest://capability-catalog`) no longer has the
  fields `compiler_source_sha256`, `backend_version` and `source_snapshot`.
  The capability registry and `get_capability` still report
  `compiler_source_sha256`; the value is now the engine source digest of the
  running build, the same value as in run identities.
- Entries in the documentation index (`qspquest://documentation`) no longer
  have `source_sha256` and `content_sha256`. The server embeds each resource
  when it is built.
- R uses the Python analysis names: `fit_data` is the native likelihood fit
  (formerly `fit_observations`). The data-frame fits are now
  `individual_fit_data_requests` and `fit_individual_data` (formerly
  `fit_data_requests` and `fit_data`).
- Analysis workflows (`fit_data`, `fit_population`, `fit_diagnostics`,
  `visual_predictive_check`, `profile_likelihood`, `bootstrap_population`,
  `exposure_summary`, `compare_regimens` and `event_sensitivities`) run in the
  native engine. They are available from R, from the CLI as
  `qspquest analyze` and from the browser engine as `analyze`. The Python
  `analysis` extra is removed; NumPy and SciPy are no longer required.
- Stochastic analyses use SplitMix64 substreams per replicate, bootstrap
  replicate or virtual subject. Rerunning an earlier analysis with the same
  seed gives results that differ within Monte Carlo error. Results are `v0.2`
  records, and checkpoints from earlier versions cannot be resumed.
- `fit_data(method=...)` accepts `"gradient"` (default) and
  `"derivative_free"` instead of `"L-BFGS-B"` and `"Powell"`.
- The population SAEM's residual-error step uses the analytic second moment
  of left-censored residuals instead of sampled latent values.
- Native parameter and regimen arguments are ordered arrays of named records;
  the Python and R functions convert their mappings.
- Fitted observation rows from `fit_data` and `fit_population` include
  `time_unit`, and Python returns fitted parameters in the declared order.
- MCP (breaking): `simulate`, `scan_parameters`, `fit_scalar` and
  `fit_individual` refuse a library model whose availability record does not
  list `exploratory_simulation`, with error code `operation_outside_supported`
  and its `preset_ids`. Such models previously accepted free-form doses and
  parameters and returned an ordinary result. Run a stored scenario instead,
  or `fork_model` the model and treat the copy as an unqualified research model.
- MCP (breaking): public and managed servers refuse a library in which any
  archive fails its checksum, path or other archive checks. Holding a failing
  archive and serving the rest applies to the local profile only.
- MCP: `simulate_steady_state` gives steady-state readouts for any model,
  nonlinear ones included; `summarize_profile` with `route`, and the `nca`
  readout with `route`, give single-dose NCA as
  `qspquest.noncompartmental_analysis` computes it, including BLQ rules;
  `scan_parameters` scans covariates and dose amounts; and dose amounts may be
  given per body mass (for example mg/kg).
- MCP builds with the `analysis` feature add an `analyze` tool for the native
  analyses, with a `plan_run` preparation that runs no simulation and draws no
  random numbers. `compare_regimens` is one operation in every interface: the
  MCP tool accepts a `population` block for paired virtual-subject comparisons,
  and without it reports the native deterministic comparison of its members.
  The native `compare_regimens` runs deterministically when `omega`,
  `random_effects` and `parameter_draws` are absent. Hosted analyses state
  `max_simulations` (at most 500) and run on one worker.
- MCP window readouts use the native analysis trapezoid; their values are
  unchanged.

## 0.1.0

- PK/PD simulation with BDF, ESDIRK34, TR-BDF2 and TSIT45 solvers.
- Scalar, individual and population fitting, sensitivity analysis and parameter scans.
- CLI, Python, R, browser and MCP interfaces using the same model and request formats.
- Native `.qsp` models, OMEX library loading and reproducible run identities.
- Installation guides and runnable simulation, fitting and library tutorials.

See each workflow’s documentation for supported estimators, units and limits.
