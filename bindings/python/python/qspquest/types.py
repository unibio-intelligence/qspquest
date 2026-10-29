"""Structural native contracts; binding validates units, ranges and support."""
from __future__ import annotations
from typing import Literal, TypedDict, Union

# Regenerate with: python scripts/generate_python_types.py

AccuracyControl = Literal['local', 'sampled']
ActiveInputs = Literal['continue', 'stop_target']
Delivery = Union['Delivery1', 'Delivery2']
DiscreteFitResultSchema = Literal['qspquest.discrete-fit-result/v0.1']
ErrorCode = Literal['invalid_input', 'unit', 'unknown_unit', 'type', 'unknown_symbol', 'expression_limit', 'unsupported', 'domain', 'invariant', 'solver', 'work_budget', 'output_budget']
FitProblem = Union['FitProblem1', 'FitProblem2', 'FitProblem3', 'FitProblem4', 'FitProblem5', 'FitProblem6']
FitResultSchema = Literal['qspquest.fit-result/v0.1']
FitSchema = Literal['qspquest.fit/v0.1']
FitStatus = Literal['converged', 'evaluation_limit', 'iteration_limit', 'line_search_failed']
GammaPoissonFitSchema = Literal['qspquest.gamma-poisson-fit/v0.1']
GaussianCensoring = Literal['left']
GaussianErrorModel = Literal['normal', 'lognormal']
InfusionSpan = Union['InfusionSpan1', 'InfusionSpan2']
MorrisProblem = Union['MorrisProblem1', 'MorrisProblem2']
MorrisResultSchema = Literal['qspquest.morris-result/v0.1']
MorrisSchema = Literal['qspquest.morris/v0.1']
ObservationSide = Literal['pre', 'post']
PeriodicTimeBasis = Literal['cycle_phase']
PopulationObjectiveKind = Union[Literal['laplace_negative_log_likelihood', 'saem_complete_data_surrogate'], Literal['focei_objective_function_value'], Literal['saem_marginal_objective_function_value']]
PopulationUncertaintyMethod = Union[Literal['focei_observed_outer_hessian'], Literal['saem_marginal_observed_hessian']]
PositiveFixedEffectIntervalMethod = Literal['positive_fixed_effect_log_wald_delta_v1']
ProbitNormalFitSchema = Literal['qspquest.probit-normal-binomial-fit/v0.1']
ProbitNormalResultSchema = Literal['qspquest.probit-normal-binomial-result/v0.1']
ResultSchema = Literal['qspquest.result/v0.1']
RunSchema = Literal['qspquest.run/v0.1']
ScalarFitResultSchema = Literal['qspquest.scalar-fit-result/v0.1']
ScalarFitSchema = Literal['qspquest.scalar-fit/v0.1']
ScanResultSchema = Literal['qspquest.scan-result/v0.1']
ScanSchema = Literal['qspquest.scan/v0.1']
SensitivityResultSchema = Literal['qspquest.sensitivity-result/v0.1']
SensitivitySchema = Literal['qspquest.sensitivity/v0.1']
Solver = Literal['diffsol_bdf', 'diffsol_tsit45', 'diffsol_esdirk34', 'diffsol_tr_bdf2', 'diffsol_rosenbrock23', 'diffsol_rodas5p', 'analytic_linear_pk']

class _AdministrationRequired(TypedDict):
    amount: Quantity
    bioavailability: float
    delivery: Delivery
    target: str
    time: Quantity

class Administration(_AdministrationRequired, total=False):
    lag: Union[Quantity, None]
    repeat: Union[Repeat, None]

class BinomialLitter(TypedDict):
    group: int
    successes: int
    total: int

class Budgets(TypedDict):
    events: int
    output_values: int
    solver_callbacks: int

class CountExposure(TypedDict):
    count: int
    exposure: Quantity

class CovariateChange(TypedDict):
    name: str
    time: Quantity
    value: Quantity

class CycleIterationRequest(TypedDict):
    absolute_tolerances: dict[str, Quantity]
    maximum_cycles: int
    relative_tolerance: float
    run: CycleIterationRequestRunRequest
    time_basis: PeriodicTimeBasis

class _CycleIterationRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class CycleIterationRequestRunRequest(_CycleIterationRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class CycleIterationResult(TypedDict):
    cycles_completed: int
    steady_state: CycleIterationResultSteadyStateResult

class CycleIterationResultSteadyStateResult(TypedDict):
    identity: ExecutionIdentity
    initial_states: dict[str, Quantity]
    maximum_scaled_residual: float
    residuals: dict[str, Quantity]

class Delivery1(TypedDict):
    kind: Literal['bolus']

class Delivery2(TypedDict):
    kind: Literal['infusion']
    span: InfusionSpan

class DiscreteFitResult(TypedDict):
    active_bounds: list[str]
    algorithm: str
    compiler_source_sha256: str
    evaluations: int
    fit_request_hash: str
    iterations: int
    likelihood: str
    negative_log_likelihood: float
    observations: int
    parameters: dict[str, Quantity]
    projected_gradient_norm: float
    schema: DiscreteFitResultSchema
    status: FitStatus

class ElementaryEffects(TypedDict):
    effects: list[float]
    mu: float
    mu_star: float
    parameter: str
    sigma: float
    unit: str

class _ErrorRequired(TypedDict):
    code: ErrorCode
    message: str

class Error(_ErrorRequired, total=False):
    expression: Union[str, None]
    time: Union[float, None]

class ExecutionIdentity(TypedDict):
    algorithm: str
    backend: str
    backend_version: str
    bound_value_hash: str
    build_profile: str
    compiler_source_sha256: str
    compiler_version: str
    model_content_hash: str
    run_request_hash: str
    rustc_version: str
    rustflags_sha256: str
    spec_version: str
    target: str

class FitBound(TypedDict):
    lower: Quantity
    parameter: str
    upper: Quantity

class FitProblem1(TypedDict):
    kind: Literal['individual']
    request: GaussianFitRequest

class FitProblem2(TypedDict):
    kind: Literal['profiled_proportional']
    request: GaussianFitRequest

class FitProblem3(TypedDict):
    kind: Literal['pooled']
    request: PooledGaussianFitRequest

class FitProblem4(TypedDict):
    kind: Literal['laplace']
    request: PopulationFitRequest

class FitProblem5(TypedDict):
    kind: Literal['focei']
    request: PopulationFitRequest

class FitProblem6(TypedDict):
    kind: Literal['saem']
    request: PopulationFitRequest

class FitRequest(TypedDict):
    problem: FitProblem
    schema: FitSchema

class _FitRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class FitRequestRunRequest(_FitRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class FitRequestSensitivityRequest(TypedDict):
    derivative_absolute_tolerances: dict[str, dict[str, Quantity]]
    run: FitRequestRunRequest
    schema: SensitivitySchema
    with_respect_to: list[str]

class _FitResultRequired(TypedDict):
    schema: FitResultSchema

class FitResult(_FitResultRequired, total=False):
    fit: Union[GaussianFitResult, None]
    population: Union[PopulationFitResult, None]
    residual_scales: Union[dict[str, float], None]

class GammaPoissonFitRequest(TypedDict):
    gradient_tolerance: float
    initial_rate: Quantity
    initial_shape: float
    max_evaluations: int
    max_iterations: int
    observations: list[CountExposure]
    rate_bounds: list[Quantity]
    schema: GammaPoissonFitSchema
    shape_bounds: list[float]

class _GaussianErrorRequired(TypedDict):
    additive_sd: Quantity
    proportional_sd: float

class GaussianError(_GaussianErrorRequired, total=False):
    model: GaussianErrorModel

class GaussianFitRequest(TypedDict):
    bounds: list[FitBound]
    gradient_tolerance: float
    max_evaluations: int
    max_iterations: int
    objective: GaussianObjectiveRequest

class GaussianFitResult(TypedDict):
    evaluations: int
    fit_request_hash: str
    iterations: int
    objective: GaussianObjectiveResult
    parameters: dict[str, Quantity]
    projected_gradient_norm: float
    status: FitStatus

class GaussianObjectiveRequest(TypedDict):
    observations: list[GaussianObservation]
    priors: list[GaussianPrior]
    simulation: FitRequestSensitivityRequest

class GaussianObjectiveResult(TypedDict):
    gradient: list[ObjectiveGradient]
    identity: ExecutionIdentity
    negative_log_likelihood: float
    negative_log_prior: float
    objective: float
    observations: int

class _GaussianObservationRequired(TypedDict):
    error: GaussianError
    output: str
    row: int
    value: Quantity

class GaussianObservation(_GaussianObservationRequired, total=False):
    censoring: Union[GaussianCensoring, None]

class GaussianPrior(TypedDict):
    mean: Quantity
    parameter: str
    sd: Quantity

class InfusionSpan1(TypedDict):
    duration: Quantity
    kind: Literal['duration']

class InfusionSpan2(TypedDict):
    kind: Literal['rate']
    rate: Quantity

class MorrisDesignProvenance(TypedDict):
    design_request_hash: str
    generator: str
    seed: int

class MorrisDesignRequest(TypedDict):
    num_levels: int
    output: str
    ranges: list[ParameterRange]
    row: int
    run: MorrisExecutionRequestRunRequest
    seed: int
    total_budgets: Budgets
    trajectory_count: int

class MorrisExecutionRequest(TypedDict):
    problem: MorrisProblem
    schema: MorrisSchema

class _MorrisExecutionRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class MorrisExecutionRequestRunRequest(_MorrisExecutionRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class MorrisExecutionRequestScanRequest(TypedDict):
    points: list[ScanPoint]
    run: MorrisExecutionRequestRunRequest
    schema: ScanSchema
    total_budgets: Budgets

class _MorrisExecutionResultRequired(TypedDict):
    result: MorrisResult
    schema: MorrisResultSchema

class MorrisExecutionResult(_MorrisExecutionResultRequired, total=False):
    design: Union[MorrisDesignProvenance, None]

class _MorrisExecutionResultRunResultRequired(TypedDict):
    description: str
    identity: ExecutionIdentity
    model_id: str
    outputs: list[ResultColumn]
    schema: ResultSchema
    sides: list[ObservationSide]
    time_unit: str
    times: list[float]

class MorrisExecutionResultRunResult(_MorrisExecutionResultRunResultRequired, total=False):
    request_id: Union[str, None]

class MorrisExecutionResultScanResult(TypedDict):
    results: list[ScanPointResult]
    scan_request_hash: str
    schema: ScanResultSchema

class MorrisProblem1(TypedDict):
    kind: Literal['explicit']
    request: MorrisRequest

class MorrisProblem2(TypedDict):
    kind: Literal['generated']
    request: MorrisDesignRequest

class MorrisRequest(TypedDict):
    num_levels: int
    output: str
    ranges: list[ParameterRange]
    row: int
    scan: MorrisExecutionRequestScanRequest
    trajectories: list[list[str]]

class MorrisResult(TypedDict):
    analysis_request_hash: str
    effects: list[ElementaryEffects]
    scan: MorrisExecutionResultScanResult

class ObjectiveGradient(TypedDict):
    parameter: str
    unit: str
    value: float

class ObservationPoint(TypedDict):
    side: ObservationSide
    time: Quantity

class ParameterRange(TypedDict):
    lower: Quantity
    parameter: str
    upper: Quantity

class PooledGaussianFitRequest(TypedDict):
    additional_subjects: list[GaussianObjectiveRequest]
    fit: GaussianFitRequest

class PopulationAdditiveErrorFit(TypedDict):
    initial: Quantity
    lower: Quantity
    output: str
    upper: Quantity

class _PopulationCovariateEffectRequired(TypedDict):
    bounds: list[float]
    covariate: str
    initial: float
    parameter: str
    reference: float
    subject_values: list[float]

class PopulationCovariateEffect(_PopulationCovariateEffectRequired, total=False):
    additional: list[PopulationCovariateEffect]

class _PopulationCovariateEffectEstimateRequired(TypedDict):
    coefficient: float
    covariate: str
    parameter: str
    reference: float

class PopulationCovariateEffectEstimate(_PopulationCovariateEffectEstimateRequired, total=False):
    additional: list[PopulationCovariateEffectEstimate]

class _PopulationFitRequestRequired(TypedDict):
    eta_bound: float
    fixed_effects: list[FitBound]
    gradient_tolerance: float
    max_evaluations: int
    max_iterations: int
    omega: list[list[float]]
    omega_diagonal_bounds: list[list[float]]
    random_effects: list[PopulationRandomEffect]
    seed: int
    subjects: list[GaussianObjectiveRequest]

class PopulationFitRequest(_PopulationFitRequestRequired, total=False):
    additive_error_fit: Union[PopulationAdditiveErrorFit, None]
    burn_in_iterations: Union[int, None]
    covariate_effect: Union[PopulationCovariateEffect, None]
    uncertainty: bool

class _PopulationFitResultRequired(TypedDict):
    evaluations: int
    fit_request_hash: str
    fixed_effects: dict[str, Quantity]
    iterations: int
    objective: float
    objective_kind: PopulationObjectiveKind
    omega: list[list[float]]
    status: FitStatus
    subjects: list[PopulationSubjectEstimate]

class PopulationFitResult(_PopulationFitResultRequired, total=False):
    covariate_effect: Union[PopulationCovariateEffectEstimate, None]
    estimated_residual_error: Union[PopulationResidualErrorEstimate, None]
    fitted_observations: Union[list[PopulationFittedObservation], None]
    fixed_effect_interval_error: Union[Error, None]
    fixed_effect_intervals: Union[PositiveFixedEffectIntervals, None]
    saem_diagnostics: Union[PopulationSaemDiagnostics, None]
    uncertainty: Union[PopulationFitUncertainty, None]

class PopulationFitUncertainty(TypedDict):
    covariance: list[list[float]]
    method: PopulationUncertaintyMethod
    parameters: list[PopulationUncertaintyParameter]
    standard_errors: list[float]

class PopulationFittedObservation(TypedDict):
    ipred: Quantity
    observation_index: int
    output: str
    pred: Quantity
    row: int
    side: ObservationSide
    subject_index: int
    time: Quantity
    value: Quantity

class PopulationRandomEffect(TypedDict):
    parameter: str

class PopulationResidualErrorEstimate(TypedDict):
    additive_sd: Quantity
    output: str

class _PopulationSaemDiagnosticsRequired(TypedDict):
    accepted_proposals: int
    burn_in_iterations: int
    consecutive_stable_iterations: int
    evaluated_proposals: int
    recent_window: int

class PopulationSaemDiagnostics(_PopulationSaemDiagnosticsRequired, total=False):
    final_relative_change: Union[float, None]
    final_step_size: Union[float, None]
    marginal_refinement_iterations: int
    post_burn_acceptance_by_axis: Union[list[float], None]
    post_burn_max_lag_one_correlation_by_axis: Union[list[float], None]
    post_burn_max_subject_stall_iterations: Union[int, None]
    post_burn_min_subject_acceptance_by_axis: Union[list[float], None]
    recent_relative_change_median: Union[float, None]

class PopulationSubjectEstimate(TypedDict):
    eta: list[float]
    negative_log_likelihood: float

class PopulationUncertaintyParameter(TypedDict):
    name: str
    unit: str

class PositiveFixedEffectInterval(TypedDict):
    estimate: float
    log_coordinate_standard_error: float
    lower: float
    name: str
    source_standard_error: float
    unit: str
    upper: float

class PositiveFixedEffectIntervals(TypedDict):
    confidence_level: float
    critical_value: float
    intervals: list[PositiveFixedEffectInterval]
    method: PositiveFixedEffectIntervalMethod

class ProbitNormalBinomialFitRequest(TypedDict):
    gradient_tolerance: float
    initial_means: list[float]
    initial_sds: list[float]
    max_evaluations: int
    max_iterations: int
    max_quadrature_evaluations: int
    mean_bounds: list[list[float]]
    observations: list[BinomialLitter]
    schema: ProbitNormalFitSchema
    sd_bounds: list[list[float]]

class ProbitNormalBinomialFitResult(TypedDict):
    active_bounds: list[int]
    algorithm: str
    compiler_source_sha256: str
    evaluations: int
    fit_request_hash: str
    iterations: int
    means: list[float]
    negative_log_likelihood: float
    observations: int
    projected_gradient_norm: float
    quadrature_evaluations: int
    schema: ProbitNormalResultSchema
    sds: list[float]
    status: FitStatus

class Quantity(TypedDict):
    unit: str
    value: float

class _RegimenRequired(TypedDict):
    administrations: list[Administration]
    end: Quantity
    rtol: float
    samples: list[Quantity]

class Regimen(_RegimenRequired, total=False):
    absolute_tolerances: Union[dict[str, Quantity], None]
    atol: Union[float, None]
    checkpoints: list[Quantity]
    covariate_changes: list[CovariateChange]
    covariates: dict[str, Quantity]
    observations: Union[list[ObservationPoint], None]
    resets: list[StateReset]
    start: Union[Quantity, None]

class Repeat(TypedDict):
    additional: int
    interval: Quantity

class ResultColumn(TypedDict):
    name: str
    unit: str
    values: list[float]

class _RunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class RunRequest(_RunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class _RunResultRequired(TypedDict):
    description: str
    identity: ExecutionIdentity
    model_id: str
    outputs: list[ResultColumn]
    schema: ResultSchema
    sides: list[ObservationSide]
    time_unit: str
    times: list[float]

class RunResult(_RunResultRequired, total=False):
    request_id: Union[str, None]

class ScalarGaussianFitRequest(TypedDict):
    absolute_tolerance: float
    lower: Quantity
    max_iterations: int
    observations: list[GaussianObservation]
    parameter: str
    run: ScalarGaussianFitRequestRunRequest
    schema: ScalarFitSchema
    upper: Quantity

class _ScalarGaussianFitRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class ScalarGaussianFitRequestRunRequest(_ScalarGaussianFitRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class ScalarGaussianFitResult(TypedDict):
    evaluations: int
    fit_request_hash: str
    iterations: int
    objective: ScalarGaussianObjectiveResult
    parameters: dict[str, Quantity]
    schema: ScalarFitResultSchema
    status: FitStatus

class ScalarGaussianObjectiveResult(TypedDict):
    identity: ExecutionIdentity
    negative_log_likelihood: float
    objective: float
    observations: int

class ScanPoint(TypedDict):
    id: str
    parameters: dict[str, Quantity]

class ScanPointResult(TypedDict):
    id: str
    parameters: dict[str, Quantity]
    result: ScanResultRunResult

class ScanRequest(TypedDict):
    points: list[ScanPoint]
    run: ScanRequestRunRequest
    schema: ScanSchema
    total_budgets: Budgets

class _ScanRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class ScanRequestRunRequest(_ScanRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class ScanResult(TypedDict):
    results: list[ScanPointResult]
    scan_request_hash: str
    schema: ScanResultSchema

class _ScanResultRunResultRequired(TypedDict):
    description: str
    identity: ExecutionIdentity
    model_id: str
    outputs: list[ResultColumn]
    schema: ResultSchema
    sides: list[ObservationSide]
    time_unit: str
    times: list[float]

class ScanResultRunResult(_ScanResultRunResultRequired, total=False):
    request_id: Union[str, None]

class SensitivityColumn(TypedDict):
    output: str
    parameter: str
    unit: str
    values: list[float]

class SensitivityRequest(TypedDict):
    derivative_absolute_tolerances: dict[str, dict[str, Quantity]]
    run: SensitivityRequestRunRequest
    schema: SensitivitySchema
    with_respect_to: list[str]

class _SensitivityRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class SensitivityRequestRunRequest(_SensitivityRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class SensitivityResult(TypedDict):
    derivatives: list[SensitivityColumn]
    result: SensitivityResultRunResult
    schema: SensitivityResultSchema

class _SensitivityResultRunResultRequired(TypedDict):
    description: str
    identity: ExecutionIdentity
    model_id: str
    outputs: list[ResultColumn]
    schema: ResultSchema
    sides: list[ObservationSide]
    time_unit: str
    times: list[float]

class SensitivityResultRunResult(_SensitivityResultRunResultRequired, total=False):
    request_id: Union[str, None]

class StateReset(TypedDict):
    active_inputs: ActiveInputs
    order: int
    target: str
    time: Quantity
    value: Quantity

class SteadyStateRequest(TypedDict):
    absolute_tolerance: Quantity
    relative_tolerance: float
    run: SteadyStateRequestRunRequest

class _SteadyStateRequestRunRequestRequired(TypedDict):
    budgets: Budgets
    regimen: Regimen
    schema: RunSchema
    solver: Solver

class SteadyStateRequestRunRequest(_SteadyStateRequestRunRequestRequired, total=False):
    accuracy_control: AccuracyControl
    initial_states: dict[str, Quantity]
    parameters: dict[str, Quantity]
    request_id: Union[str, None]
    seed: Union[int, None]

class SteadyStateResult(TypedDict):
    identity: ExecutionIdentity
    initial_states: dict[str, Quantity]
    maximum_scaled_residual: float
    residuals: dict[str, Quantity]
