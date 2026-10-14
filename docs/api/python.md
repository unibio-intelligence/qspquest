# Python API reference

Generated from Python source signatures and docstrings. Units and the scientific contract are checked by the native binder and analysis validators. Optional numerical dependencies are loaded on scientific use.

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

### CompiledSensitivities

Compile one ordered parameter selection; each run binds independent values.

Native compilation/execution release the GIL. Instances can be shared across
threads; the request's with_respect_to must match the compiled selection.
Sensitivity requests accept diffsol_bdf (default in run helpers) or explicit
diffsol_rodas5p, also used throughout individual and population fitting.

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

### model_library

```python
model_library(root=None)
```

Open the shared model catalog; Docker mounts it at QSPQUEST_LIBRARY_DIR.

The model-library loader must be installed or available on PYTHONPATH.
Archives and indexed members are verified before a named scenario runs.

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
AnalysisResult.to_frame(self)
```

Create a pandas DataFrame; requires qspquest[notebook].

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
Checkpoints contain data and random-generator state, never executable pickle.
Reuse requires identical inputs, environment and native compiler identity.
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
AnalysisControl.start(self, operation, identity, native=None)
```

See the versioned native request contract.

### AnalysisControl.boundary

```python
AnalysisControl.boundary(self, completed, total, budget, state, *, final=False)
```

See the versioned native request contract.

### restore_budget

```python
restore_budget(budget, saved)
```

Count both retained work and the native validation calls made on restart.

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
fit_data(model, run, observations_data, parameters, error, *, max_simulations=2000, max_iterations=300, tolerance=1e-07, method='L-BFGS-B', _budget=None)
```

Fit bounded parameters and additive/proportional SDs by native simulation.

Event parameters select quantity-valued paths in the run request. The normal
combined-error model adds variances. Lognormal SD is on log concentration.
All observations sharing this error must have the same unit and output.

## population

General bounded SAEM orchestrated over QSPQuest native simulations.

### fit_population

```python
fit_population(model, subjects, parameters, omega, error, *, covariates=(), covariance='full', variance_bounds=(0.0001, 2.0), seed=1009, max_iterations=600, burn_in=150, sweeps=3, tolerance=0.01, max_simulations=200000, control=None, _budget=None)
```

SAEM with 1..8 positive lognormal effects and exponential covariates.

Each selected parameter has one random effect. Each subject supplies a
native ``run``, ``observations`` and optional constant ``covariates``. Both
SDs of normal combined error can be fitted jointly with covariates. M3
censoring uses its probability in sampling and conditional latent draws
in the residual-error M step. Covariance is full or diagonal.

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
Workers (1..8, default 1) preserve ordered random streams and records.
Failed replicates remain in the denominator and suppress intervals.

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
compare_regimens(model, regimens, parameters, omega, output, *, random_effects=None, subjects=100, seed=1009, threshold=None, percentiles=(0.05, 0.5, 0.95), parameter_draws=None, max_simulations=50000)
```

Use the same virtual subjects for every regimen and return paired metrics.

``parameter_draws`` can propagate bootstrap parameter uncertainty separately
from IIV. Concentration curves exclude measurement error. Endpoint means a
trough only when the request samples the appropriate pre-dose boundary.

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

### write_analysis_report

```python
write_analysis_report(path, *, title='QSPQuest analysis', fit=None, vpc=None, comparison=None, profile=None, bootstrap=None)
```

Save a standalone report with curves, estimates, diagnostics and limits.

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
