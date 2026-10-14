//! Narrow independent Gaussian observation and prior contract.
use crate::{
    model::Quantity,
    run::{ExecutionIdentity, RunRequest, SensitivityRequest},
};
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianError {
    /// Independent additive standard deviation in the observable's units.
    pub additive_sd: Quantity,
    /// Independent proportional standard deviation, dimensionless.
    pub proportional_sd: f64,
    /// Normal errors act on the observable; lognormal errors act on its log.
    #[serde(default, skip_serializing_if = "is_normal_error")]
    pub model: GaussianErrorModel,
}
fn is_normal_error(model: &GaussianErrorModel) -> bool {
    *model == GaussianErrorModel::Normal
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GaussianErrorModel {
    #[default]
    Normal,
    Lognormal,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianObservation {
    /// Row in the simulation's ordered observation/result grid.
    pub row: usize,
    pub output: String,
    /// Measured value, or the quantification limit for a censored row.
    pub value: Quantity,
    pub error: GaussianError,
    /// M3-style observation that is known to be below its declared value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub censoring: Option<GaussianCensoring>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GaussianCensoring {
    Left,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianPrior {
    pub parameter: String,
    pub mean: Quantity,
    pub sd: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianObjectiveRequest {
    pub simulation: SensitivityRequest,
    pub observations: Vec<GaussianObservation>,
    /// Independent normal priors on natural parameter values; empty for MLE.
    pub priors: Vec<GaussianPrior>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveGradient {
    pub parameter: String,
    pub unit: String,
    pub value: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianObjectiveResult {
    pub identity: ExecutionIdentity,
    pub negative_log_likelihood: f64,
    pub negative_log_prior: f64,
    pub objective: f64,
    pub gradient: Vec<ObjectiveGradient>,
    pub observations: usize,
}

/// Finite search box in natural parameter units. Search coordinates are affine.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitBound {
    pub parameter: String,
    pub lower: Quantity,
    pub upper: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianFitRequest {
    pub objective: GaussianObjectiveRequest,
    pub bounds: Vec<FitBound>,
    pub max_evaluations: usize,
    pub max_iterations: usize,
    /// Infinity norm of the projected gradient in affine box coordinates.
    pub gradient_tolerance: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitStatus {
    Converged,
    EvaluationLimit,
    IterationLimit,
    LineSearchFailed,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianFitResult {
    pub status: FitStatus,
    pub parameters: std::collections::BTreeMap<String, Quantity>,
    pub objective: GaussianObjectiveResult,
    pub evaluations: usize,
    pub iterations: usize,
    pub projected_gradient_norm: f64,
    pub fit_request_hash: String,
}

/// Derivative-free fit for one bounded parameter when sensitivity compilation
/// would exceed the model expression budget.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ScalarFitSchema {
    #[serde(rename = "qspquest.scalar-fit/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ScalarFitResultSchema {
    #[serde(rename = "qspquest.scalar-fit-result/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarGaussianFitRequest {
    pub schema: ScalarFitSchema,
    pub run: RunRequest,
    pub parameter: String,
    pub lower: Quantity,
    pub upper: Quantity,
    pub observations: Vec<GaussianObservation>,
    pub max_iterations: usize,
    pub absolute_tolerance: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarGaussianFitResult {
    pub schema: ScalarFitResultSchema,
    pub status: FitStatus,
    pub parameters: std::collections::BTreeMap<String, Quantity>,
    pub objective: ScalarGaussianObjectiveResult,
    pub evaluations: usize,
    pub iterations: usize,
    pub fit_request_hash: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarGaussianObjectiveResult {
    pub identity: ExecutionIdentity,
    pub negative_log_likelihood: f64,
    pub objective: f64,
    pub observations: usize,
}

/// Shared-parameter fit. Priors are declared once in `fit.objective`.
/// Additional subjects supply separate regimens, observations and fixed inputs.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PooledGaussianFitRequest {
    pub fit: GaussianFitRequest,
    pub additional_subjects: Vec<GaussianObjectiveRequest>,
}

/// One population shares fixed effects and an eta covariance. Each subject
/// supplies its own regimen, covariates, observations and residual error.
/// Random effects use `individual_parameter = fixed_effect * exp(eta)`.
/// Eta and every entry of Omega are dimensionless.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationFitRequest {
    pub subjects: Vec<GaussianObjectiveRequest>,
    pub fixed_effects: Vec<FitBound>,
    pub random_effects: Vec<PopulationRandomEffect>,
    /// Covariance of the dimensionless subject eta values, in random_effects order.
    pub omega: Vec<Vec<f64>>,
    /// Lower and upper variance limits. Equal endpoints keep a variance fixed.
    pub omega_diagonal_bounds: Vec<[f64; 2]>,
    /// Absolute bound on each subject eta during conditional optimization.
    pub eta_bound: f64,
    pub max_evaluations: usize,
    pub max_iterations: usize,
    /// Laplace and FOCEI stop when normalized coordinate-search steps fall below this value.
    /// SAEM uses it in a windowed, Monte Carlo-aware drift check on the log
    /// fixed effect and log eta variance; its effective threshold is capped.
    pub gradient_tolerance: f64,
    /// Reproducible SAEM stream; ignored by deterministic Laplace and FOCEI arithmetic.
    pub seed: u64,
    /// SAEM exploration iterations before the decreasing stochastic gain.
    /// When omitted, the legacy default is one third of max_iterations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burn_in_iterations: Option<usize>,
    /// Estimate local observed-objective covariance after a converged fit.
    /// SAEM currently supports one random effect with fixed residual error;
    /// estimates at active parameter bounds are rejected.
    #[serde(default, skip_serializing_if = "is_false")]
    pub uncertainty: bool,
    /// Opt-in estimation of one common additive Gaussian SD by one-effect
    /// FOCEI or one-effect SAEM.
    /// Every observed row must use this output, zero proportional error, and
    /// declare the same starting additive SD. Other methods reject this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additive_error_fit: Option<PopulationAdditiveErrorFit>,
    /// One estimable exponential covariate effect on a one-effect SAEM model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covariate_effect: Option<PopulationCovariateEffect>,
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationRandomEffect {
    /// A selected model parameter with a shared fitted fixed effect.
    pub parameter: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationAdditiveErrorFit {
    pub output: String,
    pub initial: Quantity,
    pub lower: Quantity,
    pub upper: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationCovariateEffect {
    pub parameter: String,
    pub covariate: String,
    /// One finite, subject-constant numeric value per subject in subject order.
    pub subject_values: Vec<f64>,
    pub reference: f64,
    /// Dimensionless coefficient in `theta_i = theta_ref * exp(coefficient * (x_i-reference))`.
    pub initial: f64,
    pub bounds: [f64; 2],
    /// Additional independent, subject-constant exponential effects on the
    /// same parameter. Nested `additional` entries are invalid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional: Vec<PopulationCovariateEffect>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationSubjectEstimate {
    /// Dimensionless subject random effects in random_effects order.
    pub eta: Vec<f64>,
    /// Conditional Gaussian negative log likelihood, excluding the eta prior.
    pub negative_log_likelihood: f64,
}
/// One declared observed row projected from the final population parameters.
/// `pred` uses eta zero; `ipred` uses the fitted subject eta.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationFittedObservation {
    pub subject_index: usize,
    pub observation_index: usize,
    pub row: usize,
    pub output: String,
    pub time: Quantity,
    pub side: crate::regimen::ObservationSide,
    pub value: Quantity,
    pub pred: Quantity,
    pub ipred: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationFitResult {
    pub status: FitStatus,
    pub fixed_effects: std::collections::BTreeMap<String, Quantity>,
    pub omega: Vec<Vec<f64>>,
    /// Fitted exponential covariate coefficient, when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covariate_effect: Option<PopulationCovariateEffectEstimate>,
    pub subjects: Vec<PopulationSubjectEstimate>,
    /// Available when the method retained row-aligned final predictions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fitted_observations: Option<Vec<PopulationFittedObservation>>,
    /// Optional observed-objective covariance on natural parameter scales.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncertainty: Option<PopulationFitUncertainty>,
    /// Present when a common additive Gaussian residual SD was estimated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_residual_error: Option<PopulationResidualErrorEstimate>,
    /// Stochastic convergence evidence for SAEM only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saem_diagnostics: Option<PopulationSaemDiagnostics>,
    /// Interpretation of `objective`.
    pub objective_kind: PopulationObjectiveKind,
    pub objective: f64,
    pub evaluations: usize,
    pub iterations: usize,
    pub fit_request_hash: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationCovariateEffectEstimate {
    pub parameter: String,
    pub covariate: String,
    pub reference: f64,
    pub coefficient: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional: Vec<PopulationCovariateEffectEstimate>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationResidualErrorEstimate {
    pub output: String,
    pub additive_sd: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationSaemDiagnostics {
    pub burn_in_iterations: usize,
    /// Stochastic-approximation gain on the last completed M-step.
    pub final_step_size: Option<f64>,
    /// Largest relative change in fixed effect or eta variance on that M-step.
    pub final_relative_change: Option<f64>,
    /// Median relative M-step change over up to 20 completed post-burn-in steps.
    /// This reports stochastic jitter, not a standard error or convergence certificate.
    pub recent_relative_change_median: Option<f64>,
    pub recent_window: usize,
    pub evaluated_proposals: usize,
    pub accepted_proposals: usize,
    pub consecutive_stable_iterations: usize,
    /// Acceptance per coordinate after burn-in, including out-of-bound proposals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_burn_acceptance_by_axis: Option<[f64; 2]>,
    /// Lowest subject-level acceptance per coordinate after burn-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_burn_min_subject_acceptance_by_axis: Option<[f64; 2]>,
    /// Longest run of post-burn-in iterations without an accepted move for any subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_burn_max_subject_stall_iterations: Option<usize>,
    /// Largest positive within-subject lag-one correlation per coordinate.
    /// This is a mixing warning, not an effective sample size estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_burn_max_lag_one_correlation_by_axis: Option<[f64; 2]>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationFitUncertainty {
    /// Finite-difference Hessian of the method's observed objective, inverted
    /// as `2 * Hessian(OFV)^-1` because OFV is minus twice log likelihood.
    /// SAEM uses its Laplace-approximated marginal objective. This is local
    /// asymptotic curvature with fixed residual-error parameters; it does not
    /// include Monte Carlo variation, a sandwich correction, or interval calibration.
    pub method: PopulationUncertaintyMethod,
    /// Parameter order of both covariance axes and standard_errors.
    pub parameters: Vec<PopulationUncertaintyParameter>,
    /// Covariance on natural parameter scales; units are axis-wise products.
    pub covariance: Vec<Vec<f64>>,
    pub standard_errors: Vec<f64>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopulationUncertaintyMethod {
    FoceiObservedOuterHessian,
    /// Local curvature of SAEM's Laplace-approximated marginal objective.
    SaemMarginalObservedHessian,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationUncertaintyParameter {
    pub name: String,
    pub unit: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopulationObjectiveKind {
    LaplaceNegativeLogLikelihood,
    /// Minus twice the approximated Gaussian log likelihood, with the
    /// observation `log(2π)` constant omitted.
    FoceiObjectiveFunctionValue,
    SaemCompleteDataSurrogate,
    /// Minus twice a Laplace-approximated marginal log likelihood, omitting
    /// the observation `log(2π)` constant as FOCEI does.
    SaemMarginalObjectiveFunctionValue,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum FitSchema {
    #[serde(rename = "qspquest.fit/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum FitResultSchema {
    #[serde(rename = "qspquest.fit-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FitProblem {
    Individual(GaussianFitRequest),
    Pooled(PooledGaussianFitRequest),
    /// Bounded one-effect Laplace approximation using finite-difference conditional curvature.
    Laplace(PopulationFitRequest),
    /// Bounded FOCEI using first-order expected conditional curvature.
    /// Supports one effect or two effects with fixed correlated Omega.
    Focei(PopulationFitRequest),
    Saem(PopulationFitRequest),
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitRequest {
    pub schema: FitSchema,
    pub problem: FitProblem,
}
impl FitRequest {
    pub fn parameters(&self) -> &[String] {
        match &self.problem {
            FitProblem::Individual(r) => &r.objective.simulation.with_respect_to,
            FitProblem::Pooled(r) => &r.fit.objective.simulation.with_respect_to,
            FitProblem::Laplace(r) | FitProblem::Focei(r) | FitProblem::Saem(r) => r
                .subjects
                .first()
                .map(|s| s.simulation.with_respect_to.as_slice())
                .unwrap_or(&[]),
        }
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitResult {
    pub schema: FitResultSchema,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<GaussianFitResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<PopulationFitResult>,
}
