# Python API reference

Generated from Python source signatures and docstrings. Units and the scientific contract are checked by the native binder and analysis validators. Analysis workflows run in the native engine.

## __init__

Compile once and run versioned pharmacology requests with the native engine.

### QSPQuestError

A structured compiler or scientific execution failure with no partial result.

### QSPQuestError.__init__

```python
QSPQuestError.__init__(self, diagnostic)
```

See the versioned native request contract.

### QSPQuestError.explain

```python
QSPQuestError.explain(self)
```

Return a plain explanation and interpretation boundary.

### CompiledModel

Immutable compiled model; independent request bindings on each run.

Compilation and native execution release the GIL. A compiled model can be
shared across Python threads; returned dicts and numeric lists are independent.

### CompiledModel.__init__

```python
CompiledModel.__init__(self, source)
```

See the versioned native request contract.

### CompiledModel.model_content_hash

```python
CompiledModel.model_content_hash(self)
```

See the versioned native request contract.

### CompiledModel.output_names

```python
CompiledModel.output_names(self)
```

See the versioned native request contract.

### CompiledModel.output_units

```python
CompiledModel.output_units(self)
```

See the versioned native request contract.

### CompiledModel.morris_json

```python
CompiledModel.morris_json(self, request)
```

See the versioned native request contract.

### CompiledModel.morris

```python
CompiledModel.morris(self, request)
```

Execute explicit or seeded Morris screening with bounded scans.

### CompiledModel.scan_json

```python
CompiledModel.scan_json(self, request)
```

See the versioned native request contract.

### CompiledModel.scan

```python
CompiledModel.scan(self, request)
```

Run up to 25 explicit parameter points under aggregate budgets.

### CompiledModel.validate_run

```python
CompiledModel.validate_run(self, request)
```

Admit a fixed-regimen interchange request without integrating.

Supports diffsol_bdf, parameters and fixed bolus/infusion regimens.
Model-level lag/history/switches, covariates and per-state tolerances
require the execution API and are outside this admission contract.

### CompiledModel.run

```python
CompiledModel.run(self, request)
```

Return a qspquest.result/v0.1 dict, including execution identity.

### CompiledModel.periodic_steady_state

```python
CompiledModel.periodic_steady_state(self, request)
```

Return periodic pre-event amounts, residuals and execution identity.

### CompiledModel.iterate_periodic_state

```python
CompiledModel.iterate_periodic_state(self, request)
```

Iterate a phase-periodic model; return verified state and cycle count.

### CompiledModel.run_json

```python
CompiledModel.run_json(self, request)
```

Return the complete result as JSON for storage or interprocess transfer.

### CompiledModel.fit_scalar_json

```python
CompiledModel.fit_scalar_json(self, request)
```

Fit one bounded parameter through native simulations without sensitivities.

### CompiledModel.fit_scalar

```python
CompiledModel.fit_scalar(self, request)
```

See the versioned native request contract.

### CompiledModel.fit_data

```python
CompiledModel.fit_data(self, run, observations, parameters, error, **options)
```

Bounded likelihood fitting, including request-time event controls.

### CompiledModel.fit_population

```python
CompiledModel.fit_population(self, subjects, parameters, omega, error, **options)
```

General SAEM over native simulations; see its explicit method scope.

### CompiledModel.visual_predictive_check

```python
CompiledModel.visual_predictive_check(self, subjects, parameters, omega, error, **options)
```

See the versioned native request contract.

### CompiledModel.profile_likelihood

```python
CompiledModel.profile_likelihood(self, run, observations, parameters, error, parameter, grid, **options)
```

See the versioned native request contract.

### CompiledModel.bootstrap_population

```python
CompiledModel.bootstrap_population(self, subjects, parameters, omega, error, **options)
```

See the versioned native request contract.

### CompiledModel.compare_regimens

```python
CompiledModel.compare_regimens(self, regimens, parameters, omega, output, **options)
```

See the versioned native request contract.

### CompiledModel.event_sensitivities

```python
CompiledModel.event_sensitivities(self, run, parameters, **options)
```

See the versioned native request contract.

### fitting_identity

```python
fitting_identity(source, request, operation, *, result=None)
```

Normalize scalar/individual JSON and return native JCS identities.

This checks request/model shapes, including default fields. It does not
compile, admit a simulation, fit data, or establish scientific qualification.
Source must be model JSON; operation is fit_scalar or fit_individual.
Optional result is parsed through the matching native result type, without
asserting convergence, recovery, or scientific qualification.

### format_model

```python
format_model(source)
```

Return canonical text after validating current engine support.

### fit_gamma_poisson

```python
fit_gamma_poisson(request)
```

Fit independent count/exposure rows with a shared Gamma shape and rate.

The native bounded likelihood integrates the individual Poisson rates.
Exposure quantities must be compatible with the Gamma rate parameter unit.

### fit_probit_normal_binomial

```python
fit_probit_normal_binomial(request)
```

Fit two groups of independent binomial litters with normal probit effects.

Means and SDs are dimensionless. Native optimization uses variance
coordinates and includes the full unbounded normal mixture.

### CompiledSensitivities

Compile one ordered parameter selection; each run binds independent values.

Native compilation/execution release the GIL. Instances can be shared across
threads; the request's with_respect_to must match the compiled selection.
Sensitivity requests accept `diffsol_bdf`, the default in run helpers and
the supported solver for individual and population fitting.

### CompiledSensitivities.__init__

```python
CompiledSensitivities.__init__(self, source, with_respect_to)
```

See the versioned native request contract.

### CompiledSensitivities.run_json

```python
CompiledSensitivities.run_json(self, request)
```

See the versioned native request contract.

### CompiledSensitivities.run

```python
CompiledSensitivities.run(self, request)
```

See the versioned native request contract.

### CompiledSensitivities.fit_json

```python
CompiledSensitivities.fit_json(self, request)
```

Return a versioned fit result; inspect fit.status for convergence.

### CompiledSensitivities.plan_fit

```python
CompiledSensitivities.plan_fit(self, request)
```

Validate a native fit and return its identity/semantics without solving.

Observation rows, output dimensions, residual errors, parameter bounds
and limits are checked by the same native binder used during execution.
Admission is not convergence or statistical qualification.

### CompiledSensitivities.objective

```python
CompiledSensitivities.objective(self, request)
```

Evaluate a native GaussianObjectiveRequest and its ordered gradient.

Rows may name different endpoints with explicit endpoint-specific error
quantities. This executes sensitivities; it does not fit parameters.

### CompiledSensitivities.fit

```python
CompiledSensitivities.fit(self, request)
```

Fit an individual, pooled, or population request in native code.

### CompiledSensitivities.fit_population_data

```python
CompiledSensitivities.fit_population_data(self, dataset)
```

Fit a prepared FOCEI/SAEM dataset and retain source-row provenance.

## friendly

Small convenience interfaces that retain explicit units and native contracts.

### quantity

```python
quantity(value, unit)
```

Build a finite native quantity; the model binder validates dimensions.

### bolus

```python
bolus(target, amount, amount_unit, *, at=0.0, time_unit='h', bioavailability=1.0, lag=None)
```

Build one bolus administration with an optional explicit lag.

### infusion

```python
infusion(target, amount, amount_unit, *, duration, at=0.0, time_unit='h', bioavailability=1.0)
```

Build a fixed-duration infusion; the model must contain input forcing.

### run_request

```python
run_request(*, end, times, administrations=(), time_unit='h', side='post', solver='diffsol_bdf', parameters=None, rtol=1e-08, atol=1e-10, solver_callbacks=1000000, accuracy_control='local')
```

Build an explicit run request without changing solver or event semantics.

Sampling times must be increasing; pass native requests directly when you
need duplicate pre/post observations, resets, covariate changes, or repeats.

### scientific_status

```python
scientific_status(answer)
```

Explain result/error status and the limits on interpreting its contents.

### AnalysisResult

A dict-compatible result with summaries, long rows and optional plots.

### AnalysisResult.summary

```python
AnalysisResult.summary(self)
```

See the versioned native request contract.

### AnalysisResult.rows

```python
AnalysisResult.rows(self)
```

Return independent long-format rows, preserving times, sides and units.

### AnalysisResult.to_frame

```python
AnalysisResult.to_frame(self, table=None)
```

Create a pandas DataFrame; requires pandas (qspquest[plot]).

Without ``table``, trajectories and fitted observations give the rows
of ``rows()``. Other results, or a named ``table``, use
``qspquest.to_frame``.

### AnalysisResult.plot

```python
AnalysisResult.plot(self, output=None, *, ax=None)
```

Plot a trajectory or observed/predicted fit; requires matplotlib.

### analysis_subjects

```python
analysis_subjects(dataset, *, covariates=None)
```

Convert a PopulationFitDataset to native-simulation analysis subjects.

Keeps dosing and source-row mapping. Residual errors are supplied separately
to fit_population; endpoint error contracts remain available on the dataset.

### fit_population_dataset

```python
fit_population_dataset(model, dataset, parameters, omega, error, *, covariates_by_subject=None, **options)
```

Fit prepared NONMEM records with an explicit analysis error contract.

## control

Cooperative analysis control and verified, atomic JSON checkpoints.

### AnalysisControl

Progress callback, cooperative cancellation, and optional restart state.

``progress`` receives an independent dict after a complete work unit. Call
``cancel()`` from it or another thread. Native solves finish their bounded
call before cancellation raises ``QSPQuestError(code='cancelled')``.
Checkpoints contain JSON data written by the native analysis, never
executable pickle. Reuse requires identical inputs and native build identity.
A control is owned by one active analysis; create a new one to resume.

### AnalysisControl.__init__

```python
AnalysisControl.__init__(self, *, progress=None, checkpoint=None, resume=False, checkpoint_every=10)
```

See the versioned native request contract.

### AnalysisControl.cancel

```python
AnalysisControl.cancel(self)
```

Request cancellation at the next native-call or work-unit boundary.

### AnalysisControl.check

```python
AnalysisControl.check(self)
```

See the versioned native request contract.

### AnalysisControl.start

```python
AnalysisControl.start(self, operation, context)
```

See the versioned native request contract.

## doctor

Installation self-check runnable without compilers or optional analysis tools.

### check_environment

```python
check_environment(*, analysis=False, interchange=False)
```

Check native loading, units/events and an independent analytic trajectory.

### main

```python
main(argv=None)
```

CLI entry point: qspquest-doctor [--analysis] [--interchange] [--json].

## dataset

Explicit NONMEM-style row conversion for population fitting.

This module only builds versioned requests and preserves row provenance. The
Rust engine remains responsible for simulation, likelihoods, and estimation.

### PopulationFitDataset

A prepared request and zero-based source/result-row mapping.

### PopulationFitDataset.to_subjects

```python
PopulationFitDataset.to_subjects(self, *, covariates=None)
```

Prepare native-simulation analysis subjects with source-row provenance.

### PopulationFitDataset.fit

```python
PopulationFitDataset.fit(self, model, parameters, omega, error, *, covariates_by_subject=None, **options)
```

Fit with an explicit shared analysis residual-error contract.

### population_fit_dataset

```python
population_fit_dataset(data, template, *, compartments, endpoints, time_unit, amount_unit, rate_unit=None, observation_side='post')
```

Build a FOCEI/SAEM fit request from an explicit NONMEM record subset.

``template`` is a population fit request with one empty subject objective.
That subject supplies the sensitivity selection and run settings; records
supply doses, observation times and values for every subject.

## estimation

Bounded deterministic maximum likelihood using the native simulator.

### fit_data

```python
fit_data(model, run, observations_data, parameters, error, *, max_simulations=2000, max_iterations=300, tolerance=1e-07, method='gradient')
```

Fit bounded parameters and additive/proportional SDs by native simulation.

Event parameters select quantity-valued paths in the run request. The normal
combined-error model adds variances. Lognormal SD is on log concentration.
All observations sharing this error must have the same unit and output.
``method="gradient"`` uses projected BFGS with central-difference
gradients; ``method="derivative_free"`` uses a compass pattern search.

## population

General bounded SAEM orchestrated over QSPQuest native simulations.

### fit_population

```python
fit_population(model, subjects, parameters, omega, error, *, covariates=(), covariance='full', variance_bounds=(0.0001, 2.0), seed=1009, max_iterations=600, burn_in=150, sweeps=3, tolerance=0.01, max_simulations=200000, control=None)
```

SAEM with 1..8 positive lognormal effects and exponential covariates.

Each selected parameter has one random effect. Each subject supplies a
native ``run``, ``observations`` and optional constant ``covariates``. Both
SDs of normal combined error can be fitted jointly with covariates. M3
censoring uses its probability in sampling and the analytic conditional
second moment in the residual-error M step. Covariance is full or diagonal.

The reported objective is a complete-data surrogate, never a marginal OFV.
Convergence is a stationarity/mixing screen, not proof of a global optimum.

## diagnostics

Serializable fit diagnostics and seeded population predictive checks.

### fit_diagnostics

```python
fit_diagnostics(fit)
```

Return plotting rows, residual summaries, mixing and boundary warnings.

Standardized residuals are conditional residuals, not CWRES or NPDE.
Censored rows retain null residuals and their quantification limits.

### visual_predictive_check

```python
visual_predictive_check(model, subjects, parameters, omega, error, *, random_effects=None, covariates=(), replicates=100, bins=None, percentiles=(0.05, 0.5, 0.95), confidence=0.95, prediction_corrected=False, stratify=(), seed=1009, max_simulations=50000, control=None, workers=1)
```

Simulate population effects and observation error on the original design.

Retains each subject's regimen, fixed inputs and sampling sides. BLQ rates
are checked separately; concentration quantiles use only quantifiable values
in both observed and simulated data. Limits must be supplied for every row
for a fully censored-data-aware check. Uncertainty is Monte Carlo uncertainty
of replicate quantiles, not fitted-parameter uncertainty.
Each replicate draws from its own random substream, so results do not
depend on ``workers`` (1..8, native threads). Failed replicates remain in
the denominator and suppress intervals.

## uncertainty

Likelihood profiles and subject-resampling bootstrap with failure accounting.

### profile_likelihood

```python
profile_likelihood(model, run, observations_data, parameters, error, parameter, grid, *, confidence=0.95, max_simulations=20000, max_iterations=300)
```

Reoptimize every nuisance parameter and free SD at each fixed grid point.

Intervals are grid-interpolated likelihood-ratio approximations. Open ends
are reported as null; they are never replaced by search bounds.

### bootstrap_population

```python
bootstrap_population(model, subjects, parameters, omega, error, *, replicates=100, confidence=0.95, seed=1009, minimum_success_fraction=0.8, max_simulations=1000000, control=None, **fit_options)
```

Resample whole subjects, retaining dose histories and covariates together.

Every replicate has a distinct recorded seed, source-subject map, status,
boundary flags and failure message. Percentile intervals require the stated
success fraction. No coverage calibration is implied by finite intervals.

## exposure

Sampled-trajectory exposure summaries and paired population comparisons.

### exposure_summary

```python
exposure_summary(result, output, *, threshold=None)
```

Integrate the sampled piecewise-linear trajectory without extrapolation.

AUC and threshold duration are conditional on the supplied observation grid;
this is not terminal-phase NCA or AUC-to-infinity. Duplicate times must be
ordered pre/post so no trapezoid spans a dose jump.

### compare_regimens

```python
compare_regimens(model, regimens, parameters, omega, output, *, random_effects=None, subjects=100, seed=1009, threshold=None, percentiles=(0.05, 0.5, 0.95), parameter_draws=None, max_simulations=50000, workers=1)
```

Use the same virtual subjects for every regimen and return paired metrics.

``parameter_draws`` can propagate bootstrap parameter uncertainty separately
from IIV. Concentration curves exclude measurement error. Endpoint means a
trough only when the request samples the appropriate pre-dose boundary.
The first regimen is the reference for paired differences.

## nca

Noncompartmental analysis of observed single-dose concentration-time profiles.

Observation times are times after the dose of the same subject and occasion.
Parameter keys follow CDISC SDTM PK parameter short names (PPTESTCD).
Steady-state, multiple-dose and infusion analyses are not supported.

### noncompartmental_analysis

```python
noncompartmental_analysis(observations, doses, *, units, auc_method='linear_up_log_down', blq=None, lambda_z=None)
```

Estimate conventional single-dose NCA parameters for each subject and occasion.

``observations`` rows hold ``subject``, optional ``occasion`` (default 1),
``time`` after the dose, and either a finite ``concentration`` or
``blq=True`` with no concentration. ``doses`` rows hold ``subject``,
optional ``occasion``, ``amount`` and ``route`` (``iv_bolus`` or
``extravascular``). ``units`` declares ``time``, ``concentration`` and
``amount`` units.

Rows at or before time zero are pre-dose rows; they must be BLQ or zero.
Post-dose BLQ rows raise an error unless ``blq`` assigns an action to
leading, embedded and trailing positions. ``auc_method`` is ``linear`` or
``linear_up_log_down``. ``lambda_z`` selects the terminal points:
``{"rule": "adjusted_r2", "min_points": 3, "tolerance": 1e-4}`` (default)
or ``{"rule": "last_points", "points": n}``. Parameters that need the
terminal rate constant are ``None`` when it cannot be estimated, and the
reason is reported. Extravascular clearance and volume are apparent values
divided by an unknown bioavailability F.

AUC is reported in the UCUM product unit ``concentration.time``. Clearance
and volume are converted to ``L/time`` and ``L`` when the declared units
are recognised and dimensionally compatible (``units["derived"]`` is
``"converted"``). Otherwise they are reported without conversion in the
literal unit ``amount/(concentration.time)`` or ``amount/(concentration)``
(``"literal"``). Missing cells (None, NaN) in a ``blq`` column mean not BLQ,
and BLQ rows may hold a missing concentration.

## derivatives

Checked numerical derivatives of complete native regimens.

### event_sensitivities

```python
event_sensitivities(model, run, parameters, *, relative_step=0.001, relative_tolerance=0.02, absolute_tolerance=1e-06, max_simulations=1000)
```

Differentiate parameter-dependent lag and explicit event-time controls.

Runs the full event driver at two central-difference step sizes and reports
the half-step derivative only when they agree. Coincident observation/event
discontinuities, changing result grids, and unstable derivatives fail with
no partial derivative. This is numerical differentiation, not saltation or
symbolic moving-boundary differentiation.

## report

Portable SVG/HTML views of scientific analysis results, without network I/O.

Colours come from the figure style file shared with ``qspquest.plot``
(``plot/figure-style.json``, light mode); the report needs no plotting
packages.

### write_analysis_report

```python
write_analysis_report(path, *, title='QSPQuest analysis', fit=None, vpc=None, comparison=None, profile=None, bootstrap=None, runs=None)
```

Save a standalone report with curves, estimates, diagnostics and limits.

``comparison`` is a compare_regimens result, or a dict of them keyed by
model to overlay the models' median trajectories. ``runs`` is a dict of
labelled run results or saved tables (``qspquest.read_table``), for
example one per model; each output they share is drawn in one plot.
Colours follow the figure style shared with ``qspquest.plot``.

## interchange

Validated SBML rate-rule ODEs and the bounded QSPQuest OMEX profile.

Install ``qspquest[interchange]``. General reactions, SBML events and arbitrary
SED-ML are rejected, never silently discarded. See the interchange scope guide.

### InterchangeError

A parse, validation, integrity or unsupported-semantics failure.

``code`` distinguishes missing_dependency, invalid_document, unsupported,
integrity and resource_limit. No model or partial trajectory is returned.

### InterchangeError.__init__

```python
InterchangeError.__init__(self, code, message)
```

See the versioned native request contract.

### validate_sbml

```python
validate_sbml(text)
```

Return libSBML consistency warnings; raise for validation errors.

This validates SBML, not whether its executable semantics are supported.

### export_sbml

```python
export_sbml(model)
```

Export native model JSON or ImportedModel to SBML Level 3 Version 2.

The annotated numeric-coordinate profile preserves native units, bounds and
dosing declarations. SBML quantities themselves use numeric coordinates;
this is not a claim of portable physical unit inference by other solvers.
Text .qsp, linear_pk and event/history/covariate features are unsupported.

### ImportedModel

Native model JSON, source digest and explicit semantic/unit warnings.

### ImportedModel.compile

```python
ImportedModel.compile(self)
```

Compile using the installed native engine; no alternate solver fallback.

### import_sbml

```python
import_sbml(text)
```

Import rate-rule ODEs with parameter assignment rules.

Annotated QSPQuest documents preserve native units after verifying the SBML
against the annotation. Unannotated documents use dimensionless numeric
coordinates with explicit physical time, and report that limitation.

### ImportedArchive

Verified SBML model and named native scenarios from a bounded archive.

### ImportedArchive.run

```python
ImportedArchive.run(self, scenario_id)
```

Execute one explicitly selected scenario through the native engine.

### ImportedArchive.plan_fit

```python
ImportedArchive.plan_fit(self, fit_id)
```

Return native admission/identity for a selected archived individual fit.

### ImportedArchive.fit

```python
ImportedArchive.fit(self, fit_id)
```

Execute an admitted fit; inspect the native fit.status for convergence.

### export_omex

```python
export_omex(model, scenarios, *, fits=None)
```

Return a deterministic COMBINE archive with SBML and experiment metadata.

Accept a dict mapping scenario IDs to native run requests. The profile is
intentionally explicit and is not a general SED-ML interchange contract.
Optional named native individual ``fits`` select FIT_PROFILE. Fixed
endpoint-specific Gaussian errors are a QSPQuest extension stored as JSON;
no standard SED-ML or vendor fitting semantics are claimed.

### import_omex

```python
import_omex(data, *, expected_sha256=None)
```

Read bytes or a Path, verify inventory/hashes/profile and admit scenarios.

An optional trusted SHA-256 authenticates the entire archive. Member hashes
alone check consistency, not authorship. No extraction or network access.

## individual

Explicit paired-endpoint mapping for bounded native individual fitting.

### IndividualFitDataset

Defensive request and source-row mapping; admission does not execute a fit.

### IndividualFitDataset.request

```python
IndividualFitDataset.request(self)
```

See the versioned native request contract.

### IndividualFitDataset.mapping

```python
IndividualFitDataset.mapping(self)
```

See the versioned native request contract.

### IndividualFitDataset.plan

```python
IndividualFitDataset.plan(self, source)
```

Admit against the supplied model without numerical integration.

### IndividualFitDataset.fit

```python
IndividualFitDataset.fit(self, source)
```

Return full native result, plan and exact provenance; inspect fit.status.

### individual_fit_dataset

```python
individual_fit_dataset(data, template, *, endpoints)
```

Map long-form rows to explicit native observation indices without sorting.

Each row has exactly ``row`` (zero-based result row), ``endpoint`` (mapping
key), and ``value`` (finite measured number in the endpoint's value_unit).
Each endpoint declares exactly output, value_unit and a native Gaussian
error. Native admission checks units and likelihood support later in plan()
or fit(). No missing values, time alignment, unit inference or imputation.
The template must be an individual request with empty observations and a
fully explicit ordered observation grid; dosing and fitting controls persist.

## tables

Long-format pandas tables for QSPQuest results, with explicit unit columns.

Every table keeps the units recorded in the result. Values are not converted.
``read_table`` reads saved CSV tables with their manifests and needs only the
standard library; the other functions require pandas (``qspquest[plot]``).

### kind

```python
kind(result)
```

Return the result kind used by tables and figures, or fail if unknown.

Kinds are run, table, fit, population, diagnostics, native_fit, vpc,
profile, bootstrap, regimens, exposure, morris, scan, steady_state and
event_sensitivities.

### tables

```python
tables(result)
```

Return the table names available for a result; the first is the default.

### collection_kind

```python
collection_kind(result)
```

Return the kind of a labelled collection, or None if ``result`` is not one.

A collection is a dict whose keys are labels, or (series, panel) label
pairs, and whose values are all run results, saved tables or all regimen
comparisons.

### to_frame

```python
to_frame(result, table=None)
```

Return one long-format DataFrame for a result.

``table`` selects one of ``tables(result)``; the default is the first.
A dict that maps labels to run results or saved tables gives one
trajectory table with a ``group`` column; (series, panel) keys add a
``panel`` column. A dict that maps model labels to regimen comparisons
adds a ``model`` column. Unit columns are always included.

### read_table

```python
read_table(path)
```

Read a saved CSV table and its ``.manifest.json``.

Accepts tables written by the MCP ``save_csv`` tool, the CLI or the Python
first run (``qspquest.table-manifest/v1``). The CSV byte count and SHA-256,
header, row and event counts, finite values and the pre-then-post order of
equal times are checked against the manifest. Units, column meanings and
model identity are taken from the manifest; values are not converted.
Outputs are named by their native model name where the manifest records
one, otherwise by the CSV column. The returned table plots and converts
like a run result, and the full manifest is kept under ``manifest``. Receipts in the manifest are kept but
not re-validated here.

### wilson_interval

```python
wilson_interval(successes, total, confidence=0.95)
```

Wilson score interval for a binomial proportion.

### target_attainment

```python
target_attainment(result, metric, target, *, direction='below', confidence=0.95)
```

Fraction of virtual subjects whose metric meets a target, per regimen.

``result`` is a compare_regimens result or a dict of them keyed by model.
``metric`` is a per-subject metric (auc, cmax, cmin, tmax, endpoint or
time_above_threshold). ``direction='below'`` counts subjects with
metric <= target and ``'above'`` those with metric >= target. Intervals
are Wilson score intervals. Metrics cover the sampled window of the
comparison, so a window maximum below a target is ``cmax`` with
``direction='below'``.

## plot

Publication figures for QSPQuest results.

Two layers make up this package. ``qspquest.plot.style`` holds colour definitions,
the theme, scales and journal-size export; it does not depend on
pharmacometric results, and every colour comes from its ``figure-style.json``.
``qspquest.plot.pkpd`` builds figures from QSPQuest results with that layer.
``theme_qspquest`` and the ``scale_*_qspquest`` functions are the style
layer's ``theme_style`` and ``scale_*`` functions under QSPQuest names. The data
behind every figure are available from ``qspquest.to_frame``.
Requires ``qspquest[plot]`` (pandas and plotnine).

## plot.style

Figure style: colour definitions, theme, scales and journal-size export.

The layer is independent of pharmacometric results. Every colour, line
weight, font list and journal preset is read from ``figure-style.json`` next to this
file; no colour is defined in code. Requires pandas and plotnine.

### StyleError

A style request that the style definitions or journal presets cannot satisfy.

### definitions

```python
definitions()
```

Return a copy of all style definitions.

### modes

```python
modes()
```

Return the available colour modes.

### set_mode

```python
set_mode(name)
```

Select the colour mode ('light' or 'dark') for figures built afterwards.

### mode

```python
mode()
```

Return the current colour mode.

### color

```python
color(name)
```

Return one named colour: surface, ink, muted, grid or fill.

### palette

```python
palette(n=None)
```

Return the first ``n`` categorical colours (all by default), in their fixed order.

### sequential

```python
sequential()
```

Return the single-hue sequential ramp, from the surface towards high contrast.

### diverging

```python
diverging()
```

Return the diverging ramp; the middle colour is the neutral midpoint.

### line

```python
line(name)
```

Return a plotnine line size for a named weight (grid, edge, reference, model, interval).

### point_size

```python
point_size()
```

Return the plotnine point size.

### fill_alpha

```python
fill_alpha()
```

Return the opacity of shaded intervals and ribbons.

### journals

```python
journals()
```

Return the journal presets (widths and heights in mm, type in pt).

### type_sizes

```python
type_sizes(journal=None)
```

Return the journal's (base, small) type sizes in pt.

### font

```python
font(journal=None)
```

Return the first installed font of the journal's preferred list.

### theme_style

```python
theme_style(base_size=None, small_size=None, *, journal=None)
```

Surface-coloured theme with a recessive grid and the legend below the panel.

Type sizes and the font follow the journal preset unless given.

### scale_color_categorical

```python
scale_color_categorical(**kwargs)
```

Categorical colour scale in the fixed palette order.

### scale_fill_categorical

```python
scale_fill_categorical(**kwargs)
```

Categorical fill scale in the fixed palette order.

### scale_fill_sequential

```python
scale_fill_sequential(**kwargs)
```

Single-hue sequential fill for magnitudes.

### scale_color_sequential

```python
scale_color_sequential(**kwargs)
```

Sequential colour for lines and points; omits the two steps nearest the surface.

### scale_fill_diverging

```python
scale_fill_diverging(**kwargs)
```

Diverging fill with a neutral midpoint; set ``limits`` symmetric about it.

### size

```python
size(fig, width_mm, height_mm)
```

Set the intended figure size in mm; ``save`` keeps this aspect ratio.

### save

```python
save(fig, path, *, width=None, height=None, journal=None, formats=None, dpi=600, svg_text='paths')
```

Write a figure as PDF, SVG and PNG at a journal column width.

``path`` is a file name; its suffix selects one format, and a name without
a suffix writes ``formats`` (default PDF, SVG and PNG). ``width`` is
'single-column', 'double-column' or a width in mm; by default figures
designed for one column (up to 100 mm) use the single-column width and
others the double-column width. ``height`` (mm) defaults to the figure's
own aspect ratio, limited to the journal's maximum figure height. Type
sizes and font follow the journal preset.

PDF fonts are embedded as TrueType (Type 42) and the text stays editable.
SVG text is drawn as glyph outlines by default (``svg_text='paths'``), so
the file looks the same without the font; ``svg_text='text'`` keeps
editable text that needs the font on the reader's computer. Creation
dates and software names are omitted from file metadata, so a rerun with
the same inputs, plotnine and Matplotlib versions and fonts gives the same
files. Returns the paths.

## plot.pkpd

Pharmacometric figures built from QSPQuest results.

Each function takes a native QSPQuest result, reads axis labels and units from
that result, and returns a plotnine object that can be extended with ``+``.
Colours, line weights and sizes come only from the public API of
``qspquest.plot.style``.

### profile

```python
profile(result, output=None, *, log=False, label=None, observed=None, subjects=None, time_unit=None)
```

Concentration-time (or any output-time) profile.

Accepts a run result, a saved table from ``qspquest.read_table``, a dict of
labelled runs or tables (one line each), a scan result (one line per
point), or a fit, population fit, diagnostics or native population fit
(observations as points, predictions as lines, one panel per subject).
Dict keys may be (series, panel) pairs, for example (model, regimen):
lines are then coloured by the first element and drawn in one panel per
second element. ``observed`` adds points from a DataFrame with columns
``time``, ``value``, ``unit`` and ``time_unit`` (and optionally ``group``).
Up to three series are coloured; more series are drawn in separate panels.

### observed_vs_predicted

```python
observed_vs_predicted(result, *, prediction=None, output=None, log=False, label=None)
```

Observed versus predicted values with the line of identity.

``prediction`` is 'individual' (default) or 'population' for population
fits; individual fits have one model prediction. Rows below the LLOQ are
omitted and counted in a warning.

### residuals

```python
residuals(result, *, x='time', standardized=None, output=None, time_unit=None)
```

Residuals against time or a prediction, with a zero reference line.

Standardized residuals are the conditional residuals recorded by the fit
(residual divided by the residual-error SD at the prediction); they are
not CWRES or NPDE. Dashed lines mark ±1.96. Native population fits record
no standardized residuals; their raw residuals are observed minus
individual prediction. ``x`` is 'time', 'prediction' or
'individual_prediction'.

### goodness_of_fit

```python
goodness_of_fit(result, *, output=None, log=False, time_unit=None)
```

Standard goodness-of-fit panels, labelled A–D.

Population fits: observed vs population and individual predictions, and
residuals vs time and vs the prediction the residual is computed against.
Individual fits have three panels. Returns a plotnine composition.

### vpc

```python
vpc(result, *, log=False, label=None)
```

Visual predictive check, or prediction-corrected VPC.

Shaded boxes span each time bin and show the Monte Carlo confidence
interval of each simulated percentile; points joined by lines are the
observed percentiles at bin midpoints. Intervals are absent when any
replicate failed; the simulated percentiles are then drawn as dashed
lines and a warning is issued.

### vpc_blq

```python
vpc_blq(result)
```

Fraction of rows below the LLOQ in each time bin.

The line joins the observed fractions; shaded boxes are the confidence
intervals of the simulated fractions.

### parameters

```python
parameters(result, *, estimates=None)
```

Parameter estimates with confidence intervals, relative to the estimate.

Accepts a bootstrap, likelihood profile or native population fit with
fixed-effect intervals. Each row is labelled with its estimate, interval
and unit; the x axis is the ratio to the estimate on a log scale. A bound
the method did not reach is reported as 'not reached'. For a bootstrap,
``estimates`` (the original fit) supplies the point estimates; otherwise
the bootstrap median is used.

### bootstrap

```python
bootstrap(result, *, estimates=None, bins=20)
```

Distribution of converged bootstrap estimates, one panel per parameter.

Dashed lines mark the percentile interval; the solid line marks the
original estimate when ``estimates`` is given, otherwise the median.

### likelihood_profile

```python
likelihood_profile(result)
```

Profile of the negative log-likelihood for one parameter.

The dashed horizontal line is the likelihood-ratio threshold; dotted
vertical lines mark the interval bounds the grid reached. Grid points that
did not converge are omitted and counted in a warning.

### regimens

```python
regimens(result, *, log=False, label=None, target=None)
```

Percentile bands of simulated profiles for each regimen.

The line is the requested percentile nearest 0.5 (the median when
requested), and the band spans the outermost requested percentiles.
Concentration curves exclude measurement error.
``result`` may also be a dict of comparisons keyed by model, for example
from two model variants with the same seed and regimens: models are then
coloured and each regimen has its own panel. ``target`` draws a dashed
reference line in the output unit.

### target_attainment

```python
target_attainment(result, metric, target, *, direction='below', confidence=0.95)
```

Probability of target attainment per regimen, with Wilson intervals.

Counts the virtual subjects of a compare_regimens result whose ``metric``
is at most (``direction='below'``) or at least (``'above'``) ``target``,
in the metric's unit. Bars are Wilson score intervals at ``confidence``.
A dict of comparisons keyed by model overlays the models on the same
regimens. Regimens are listed in comparison order. The data are
``qspquest.tables.target_attainment`` with the same arguments.

### exposure

```python
exposure(result, metric='auc', *, paired=True, log=False)
```

Per-subject exposure metric for each regimen.

Points are virtual subjects; grey lines join the same subject across
regimens when ``paired`` is true; the bar marks the median.

### exposure_ratio

```python
exposure_ratio(result, metrics=('auc', 'cmax'), *, bounds=(0.8, 1.25))
```

Paired ratio of each regimen to the reference regimen, per metric.

For every virtual subject the metric under a regimen is divided by the
same subject's metric under the reference. The point is the median ratio
and the bar spans the comparison's outermost percentiles. ``bounds`` draws
a shaded reference range (set None to omit).

### random_effects

```python
random_effects(result, *, covariates=None, bins=15)
```

Distribution of individual random effects (η), one panel per effect.

Panel titles report the η-variance shrinkage recorded by the fit. With
``covariates`` (a DataFrame with a ``subject`` column), η is plotted
against each numeric covariate instead.

### trace

```python
trace(result)
```

Parameter estimates over SAEM iterations or optimizer simulations.

### morris

```python
morris(result, *, label=None)
```

Morris screening: μ* (mean absolute elementary effect) against σ.

Points far from the origin along μ* have a large overall effect; a large
σ relative to μ* indicates interaction or nonlinearity.

### scan

```python
scan(result, output=None, *, metric='max', label=None, log=None)
```

Parameter scan: trajectories coloured by one varied parameter, or a heat map.

With one varied parameter, each point's trajectory is drawn and coloured
on a sequential scale. With two, each tile shows ``metric`` ('max', 'min',
'last' or 'auc' over the sampled window) for one combination. ``log``
selects logarithmic parameter axes; by default they are logarithmic when
the scanned range spans a factor of 10 or more.

### steady_state

```python
steady_state(result)
```

Pre-cycle amounts at periodic steady state, one row per state.

Amounts are the states immediately before the cycle's start-time events.
States with different units are drawn in separate panels.
