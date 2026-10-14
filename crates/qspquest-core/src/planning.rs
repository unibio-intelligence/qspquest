//! Static preparation records. A plan contains no integrated trajectory or qualification claim.
use crate::{regimen::ObservationSide, run::*, Budgets, Event};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactUnit {
    /// SI kg, m, s, mol, K, A, cd exponents, followed by opaque activity.
    pub dimensions: [i16; 8],
    /// Nonzero opaque entity exponents, keyed by cell or genome. Empty for legacy units.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "entity_dimensions_schema"))]
    pub entities: BTreeMap<crate::units::Entity, i16>,
    /// Positive exact rational scale to the dimension's base coordinates.
    /// Decimal strings preserve integers beyond JSON's exact numeric range.
    pub scale_numerator: String,
    pub scale_denominator: String,
    /// Original declared spelling, when the unit was not inferred.
    pub declared: Option<String>,
}

#[cfg(feature = "schema")]
fn entity_dimensions_schema(
    generator: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject};
    let exponent = generator.subschema_for::<i16>();
    SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(ObjectValidation {
            properties: BTreeMap::from([
                ("cell".into(), exponent.clone()),
                ("genome".into(), exponent),
            ]),
            additional_properties: Some(Box::new(Schema::Bool(false))),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    Parameter,
    Covariate,
    IndividualParameter,
    DerivedCovariate,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundInput {
    pub name: String,
    pub kind: BindingKind,
    pub value: f64,
    pub unit: ExactUnit,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingActivation {
    Initial,
    PostBoundary,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingPhase {
    /// Uses RunPlan.time_unit. Initial and post-boundary phases can share a time.
    pub time: f64,
    pub activation: BindingActivation,
    pub inputs: Vec<BoundInput>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedState {
    pub name: String,
    pub unit: String,
    /// Actual integration initial value, before boundary updates and dosing.
    pub initial: f64,
    pub absolute_tolerance: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedOutput {
    pub name: String,
    pub unit: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedEvent {
    /// Driver storage order is preserved. Reset.order remains the reset ordering key.
    /// All event times and infusion durations use RunPlan.time_unit.
    pub event: Event,
    pub target_name: String,
    /// Reset value uses state units; delivered dose amount uses input units before model scaling.
    pub quantity_unit: ExactUnit,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedObservation {
    pub time: f64,
    pub side: ObservationSide,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadoutInputOverrides {
    pub observation: usize,
    /// Exact pointwise predicate inputs, before any readout expression is evaluated.
    pub covariates: BTreeMap<String, crate::model::Quantity>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWorkload {
    pub states: usize,
    pub outputs: usize,
    pub events: usize,
    /// Events plus binding/checkpoint discontinuities, as charged by driver admission.
    pub reserved_events: usize,
    pub binding_phases: usize,
    pub driver_rows: usize,
    pub result_rows: usize,
    /// Conservative reservation used by admission, including binding storage.
    pub reserved_binding_values: usize,
    pub stored_state_values: usize,
    pub stored_output_values: usize,
    pub readout_override_values: usize,
    /// Ceilings, not predicted runtime or callback counts.
    pub budgets: Budgets,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeferredCheck {
    TrajectoryDomain,
    ReadoutDomain,
    NumericalConvergence,
    IndependentScientificQualification,
    Optimization,
    Identifiability,
    ObservedCovariance,
    ElementaryEffectOverflow,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum RunPlanSchema {
    #[serde(rename = "qspquest.run-plan/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPlan {
    pub schema: RunPlanSchema,
    pub request: RunRequest,
    /// Preserve None versus an explicit ordered selection in the scientific identity.
    pub selected_outputs: Option<Vec<String>>,
    /// Computed by execution's identity function; does not prove execution occurred.
    pub prospective_identity: ExecutionIdentity,
    pub time_unit: String,
    pub start: f64,
    pub end: f64,
    pub relative_tolerance: f64,
    pub states: Vec<PlannedState>,
    pub outputs: Vec<PlannedOutput>,
    pub binding_phases: Vec<BindingPhase>,
    pub events: Vec<PlannedEvent>,
    pub checkpoints: Vec<f64>,
    pub driver_rows: Vec<PlannedObservation>,
    /// Requested order, duplicates and sides are preserved.
    pub observations: Vec<PlannedObservation>,
    /// Requested observation -> driver row. A smooth row can serve either side.
    pub observation_rows: Vec<usize>,
    pub readout_overrides: Vec<ReadoutInputOverrides>,
    pub workload: RunWorkload,
    pub deferred_checks: Vec<DeferredCheck>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ScanPlanSchema {
    #[serde(rename = "qspquest.scan-plan/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanPointPlan {
    pub id: String,
    pub parameters: BTreeMap<String, crate::model::Quantity>,
    pub run: RunPlan,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanPlan {
    pub schema: ScanPlanSchema,
    pub scan_request_hash: String,
    pub total_budgets: Budgets,
    pub points: Vec<ScanPointPlan>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MorrisPlanSchema {
    #[serde(rename = "qspquest.morris-plan/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisRangePlan {
    pub parameter: String,
    pub lower: f64,
    pub upper: f64,
    pub unit: ExactUnit,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisStepPlan {
    pub from_point: String,
    pub to_point: String,
    pub parameter: String,
    pub normalized_delta: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisPlan {
    pub schema: MorrisPlanSchema,
    pub request: crate::morris::MorrisExecutionRequest,
    pub analysis_request_hash: String,
    pub design: Option<crate::morris::MorrisDesignProvenance>,
    pub ranges: Vec<MorrisRangePlan>,
    pub trajectories: Vec<Vec<String>>,
    pub steps: Vec<MorrisStepPlan>,
    pub output: PlannedOutput,
    pub row: usize,
    pub observation: PlannedObservation,
    pub scan: ScanPlan,
    pub deferred_checks: Vec<DeferredCheck>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum PeriodicPlanSchema {
    #[serde(rename = "qspquest.periodic-plan/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PeriodicPlanRequest {
    SteadyState(Box<SteadyStateRequest>),
    IteratePeriodic(Box<CycleIterationRequest>),
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeriodicStatePlan {
    pub name: String,
    pub unit: String,
    /// None for the affine fixed-point solve; iteration retains the supplied seed.
    pub seed: Option<f64>,
    pub convergence_absolute_tolerance: f64,
    /// None for the analytic propagator; its numerical method does not use a local solver tolerance.
    pub integration_absolute_tolerance: Option<f64>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeriodicPlan {
    pub schema: PeriodicPlanSchema,
    pub request: PeriodicPlanRequest,
    pub prospective_identity: ExecutionIdentity,
    pub inputs: Vec<BoundInput>,
    pub time_unit: String,
    /// Exact normalized phase cycle, including folded infusion carryover.
    pub cycle: crate::Protocol,
    /// In cycle.events order. Affine solver doses have already been scaled into state amounts.
    pub event_quantity_units: Vec<ExactUnit>,
    pub states: Vec<PeriodicStatePlan>,
    pub convergence_relative_tolerance: f64,
    /// n+2 for the affine solve; maximum_cycles for BDF iteration.
    pub maximum_cycle_executions: usize,
    pub total_budgets: Budgets,
    pub per_cycle_budgets: Budgets,
    pub deferred_checks: Vec<DeferredCheck>,
    /// Neither admission nor convergence from a seed establishes uniqueness or stability.
    pub fixed_point_uniqueness_and_stability_certified: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AugmentedStatePlan {
    pub state: String,
    /// None is the primal state; derivatives follow selected parameter-major order.
    pub parameter: Option<String>,
    pub unit: ExactUnit,
    pub initial: f64,
    pub absolute_tolerance: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivativeOutputPlan {
    pub output: String,
    pub parameter: String,
    pub unit: ExactUnit,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SensitivityPlanSchema {
    #[serde(rename = "qspquest.sensitivity-plan/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivityPlan {
    pub schema: SensitivityPlanSchema,
    pub request: SensitivityRequest,
    pub prospective_identity: ExecutionIdentity,
    pub time_unit: String,
    pub start: f64,
    pub end: f64,
    pub relative_tolerance: f64,
    pub augmented_states: Vec<AugmentedStatePlan>,
    pub outputs: Vec<PlannedOutput>,
    pub derivatives: Vec<DerivativeOutputPlan>,
    pub binding_phases: Vec<BindingPhase>,
    pub events: Vec<PlannedEvent>,
    pub checkpoints: Vec<f64>,
    pub driver_rows: Vec<PlannedObservation>,
    pub observations: Vec<PlannedObservation>,
    pub observation_rows: Vec<usize>,
    /// State/output counts include derivative coordinates/columns.
    pub workload: RunWorkload,
    pub deferred_checks: Vec<DeferredCheck>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianObservationPlan {
    pub row: usize,
    pub output: String,
    pub unit: String,
    pub value: f64,
    pub additive_sd: f64,
    /// Dimensionless relative SD for normal errors; log SD for lognormal errors.
    pub proportional_sd: f64,
    pub model: crate::fit::GaussianErrorModel,
    pub censoring: Option<crate::fit::GaussianCensoring>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianPriorPlan {
    pub parameter: String,
    pub unit: String,
    pub mean: f64,
    pub sd: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum GaussianObjectivePlanSchema {
    #[serde(rename = "qspquest.gaussian-objective-plan/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianObjectivePlan {
    pub schema: GaussianObjectivePlanSchema,
    pub request: crate::fit::GaussianObjectiveRequest,
    /// Full objective identity, including observations/priors and the original budget.
    pub prospective_identity: ExecutionIdentity,
    /// Actual simulation preparation after reserving observation/prior storage.
    pub simulation: SensitivityPlan,
    pub observations: Vec<GaussianObservationPlan>,
    pub priors: Vec<GaussianPriorPlan>,
    pub reserved_objective_values: usize,
    /// No predictions, likelihood, gradients, convergence or qualification are certified.
    pub prediction_dependent_checks_deferred: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum FitPlanSchema {
    #[serde(rename = "qspquest.fit-plan/v0.1")]
    V01,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitSearchCoordinate {
    AffineUnitBox,
    LogNaturalValue,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitCoordinatePlan {
    pub parameter: String,
    pub unit: ExactUnit,
    pub initial: f64,
    pub lower: f64,
    pub upper: f64,
    pub search_coordinate: FitSearchCoordinate,
    /// Actual transformed initial search value; log uses the numeric value in the declared unit.
    pub initial_search_value: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualParameterization {
    StandardDeviation,
    Variance,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualFitPlan {
    pub output: String,
    /// Observable units; variance coordinates use their square.
    pub output_unit: String,
    pub parameterization: ResidualParameterization,
    pub initial: f64,
    pub lower: f64,
    pub upper: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationPreparationPlan {
    /// Dimensionless eta covariance in selected random-effect order.
    pub omega: Vec<Vec<f64>>,
    pub omega_diagonal_bounds: Vec<[f64; 2]>,
    pub eta_bound: f64,
    pub burn_in_iterations: Option<usize>,
    /// SAEM centered covariate design, in subject order; empty without fitted covariates.
    pub centered_covariates: Vec<Vec<f64>>,
    pub initial_covariate_coefficients: Vec<f64>,
    pub residual: Option<ResidualFitPlan>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimalLikelihoodPlan {
    /// Subject-specific admission baseline, including initial covariate effects.
    pub request: crate::fit::GaussianObjectiveRequest,
    pub simulation: RunPlan,
    pub observations: Vec<GaussianObservationPlan>,
    pub reserved_objective_values: usize,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoceiLikelihoodPlan {
    pub simulation: SensitivityPlan,
    pub observations: Vec<GaussianObservationPlan>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "preparation",
    content = "plan",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FitSubjectPlan {
    GaussianObjective(Box<GaussianObjectivePlan>),
    PrimalLikelihood(Box<PrimalLikelihoodPlan>),
    FoceiLikelihood(Box<FoceiLikelihoodPlan>),
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitPlan {
    pub schema: FitPlanSchema,
    pub request: crate::fit::FitRequest,
    /// Canonical original input and model/compiler/backend context. Not a final fit-result identity.
    pub fit_input_hash: String,
    /// Canonical complete plan with this field set to the empty string.
    pub plan_hash: String,
    pub coordinates: Vec<FitCoordinatePlan>,
    pub population: Option<PopulationPreparationPlan>,
    /// Preparation at admitted starts, never future optimizer trials or sampled eta values.
    pub subjects: Vec<FitSubjectPlan>,
    pub max_evaluations: usize,
    pub max_iterations: usize,
    pub gradient_tolerance: f64,
    /// Independent aggregate ceiling for conservative descriptive-record charges, not numerical work.
    pub metadata_item_limit: usize,
    pub metadata_items: usize,
    pub deferred_checks: Vec<DeferredCheck>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ScalarFitPlanSchema {
    #[serde(rename = "qspquest.scalar-fit-plan/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarFitPlan {
    pub schema: ScalarFitPlanSchema,
    pub request: crate::fit::ScalarGaussianFitRequest,
    pub fit_input_hash: String,
    pub plan_hash: String,
    pub parameter: String,
    pub unit: ExactUnit,
    pub lower: f64,
    pub upper: f64,
    /// First golden-section trial; the simulation below describes original-input admission.
    pub first_trial: f64,
    pub admission: RunPlan,
    pub observations: Vec<GaussianObservationPlan>,
    /// Implicit trajectory grids can change with a fitted dosing lag. Trials check these rows.
    pub observation_row_checks_deferred: bool,
    pub max_iterations: usize,
    /// In the parameter's declared natural units.
    pub absolute_tolerance: f64,
    pub metadata_item_limit: usize,
    pub metadata_items: usize,
    pub deferred_checks: Vec<DeferredCheck>,
}
