//! Pharmacology execution engine. Native simulation and bounded fitting API.
pub mod capabilities;
pub mod compile;
pub mod discrete_fit;
pub mod document;
pub mod hmm;
pub mod language;
pub mod linear_pk;
pub mod probit_normal;
pub mod runtime;
pub mod steady_state;
use diffsol::AdaptiveLU;
use diffsol::{NalgebraMat, OdeBuilder, OdeSolverMethod};
use qspquest_core::run::Solver;
pub use qspquest_core::{Budgets, Error, ErrorCode, Event, Protocol, Sample};
use std::cell::{Cell, RefCell};

/// Exact vendored Diffsol source revision, including both local Rosenbrock methods.
pub(crate) const DIFFSOL_BACKEND_VERSION: &str = concat!(
    env!("QSPQUEST_DIFFSOL_VERSION"),
    "+sha256:",
    env!("QSPQUEST_DIFFSOL_SOURCE_SHA256")
);

/// Constant-coefficient interval propagation used by the common event driver.
/// The driver retains ownership of event ordering, observations and budgets.
pub trait IntervalPropagator {
    fn propagate(&self, amounts: &[f64], rates: &[f64], duration: f64) -> Result<Vec<f64>, Error>;
}

/// Models supply equations and directional Jacobian products; the event driver
/// supplies rates in the model state/time basis.
pub trait Model {
    /// Conservative structural Jacobian dependencies, including entries zero at initialization.
    fn jacobian_sparsity(&self) -> Option<Vec<(usize, usize)>> {
        None
    }
    /// Conservative default for callbacks; compiled equations can prove autonomy.
    fn rhs_depends_on_time(&self) -> bool {
        true
    }

    /// Opt in only when this propagator describes the complete current model.
    fn interval_propagator(&self) -> Option<&dyn IntervalPropagator> {
        None
    }

    fn check_delivery(&self, _event: &Event) -> Result<(), Error> {
        Ok(())
    }
    fn validate(&self) -> Result<(), Error>;
    /// Binding discontinuities are fixed in advance and never inferred from solver callbacks.
    fn discontinuities(&self) -> &[f64] {
        &[]
    }
    /// Times where the ODE right-hand side changes. A model may have additional
    /// smooth checkpoints in `discontinuities` that still split integration.
    fn rhs_jump_times(&self) -> &[f64] {
        self.discontinuities()
    }
    /// Called after the pre-event state is captured, before resets and doses.
    fn enter_boundary(&self, _time: f64) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64>;
    /// Validate a physical state after accepted steps, dense output and discrete
    /// updates. This deliberately excludes Newton iterates and rejected steps.
    fn check_state(&self, _time: f64, _state: &[f64]) -> Result<(), Error> {
        Ok(())
    }

    fn dose_scale(&self, state: usize) -> Option<f64>;
    /// Apply a fixed-time bolus, including any augmented-state jump derivatives.
    fn apply_bolus(
        &self,
        _time: f64,
        target: usize,
        amount: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        let scale = self
            .dose_scale(target)
            .ok_or_else(|| Error::new(ErrorCode::InvalidInput, "unsupported bolus target"))?;
        *state
            .get_mut(target)
            .ok_or_else(|| Error::new(ErrorCode::InvalidInput, "invalid bolus target"))? +=
            amount * scale;
        Ok(())
    }
    /// Constant state resets have zero parameter derivative for the reset state.
    fn apply_reset(
        &self,
        _time: f64,
        target: usize,
        value: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        *state
            .get_mut(target)
            .ok_or_else(|| Error::new(ErrorCode::InvalidInput, "invalid reset target"))? = value;
        Ok(())
    }
    /// Receive the unscaled input rate so zero dose scales retain their derivatives.
    fn accumulate_infusion_rate(
        &self,
        _time: f64,
        target: usize,
        rate: f64,
        rates: &mut [f64],
    ) -> Result<(), Error> {
        let scale = self
            .dose_scale(target)
            .ok_or_else(|| Error::new(ErrorCode::InvalidInput, "unsupported infusion target"))?;
        *rates
            .get_mut(target)
            .ok_or_else(|| Error::new(ErrorCode::InvalidInput, "invalid infusion target"))? +=
            rate * scale;
        Ok(())
    }
    fn rhs(&self, time: f64, x: &[f64], rate: &[f64], out: &mut [f64]) -> Result<(), Error>;
    fn jac_mul(
        &self,
        time: f64,
        x: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error>;
}

fn sorted_unique(values: &mut Vec<f64>) {
    values.sort_by(f64::total_cmp);
    values.dedup();
}

fn incoming_f64(time: f64) -> f64 {
    if time == 0.0 {
        -f64::from_bits(1)
    } else if time > 0.0 {
        f64::from_bits(time.to_bits() - 1)
    } else {
        f64::from_bits(time.to_bits() + 1)
    }
}

trait ProtocolExt {
    fn validate<M: Model>(&self, model: &M) -> Result<(), Error>;
    fn boundaries(&self) -> Vec<f64>;
    fn has_event(&self, t: f64) -> bool;
}
impl ProtocolExt for Protocol {
    fn validate<M: Model>(&self, model: &M) -> Result<(), Error> {
        model.validate()?;
        if !self.start.is_finite()
            || !self.end.is_finite()
            || self.end <= self.start
            || !self.rtol.is_finite()
            || !self.atol.is_finite()
            || self.rtol <= 0.0
            || self.atol <= 0.0
        {
            return Err("end and tolerances must be finite and positive".into());
        }
        let initial = model.initial();
        model.check_state(self.start, &initial)?;
        if initial.is_empty() || initial.iter().any(|v| !v.is_finite()) {
            return Err("invalid initial state".into());
        }
        if self
            .samples
            .iter()
            .any(|t| !t.is_finite() || *t < self.start || *t > self.end)
        {
            return Err("sample outside simulation window".into());
        }
        let mut reset_orders = std::collections::BTreeSet::new();
        for e in &self.events {
            if !e.time().is_finite()
                || e.time() < self.start
                || e.time() > self.end
                || e.target() >= initial.len()
            {
                return Err("invalid dose".into());
            }
            if let Event::Reset {
                value, time, order, ..
            } = e
            {
                if !value.is_finite() {
                    return Err("reset value must be finite".into());
                }
                if !reset_orders.insert((if *time == 0.0 { 0 } else { time.to_bits() }, *order)) {
                    return Err("simultaneous resets require distinct order values".into());
                }
                continue;
            }
            model.check_delivery(e)?;
            let amount = e.amount().expect("dose variant");
            if !amount.is_finite() || amount < 0.0 {
                return Err("invalid dose amount".into());
            }
            let scale = model
                .dose_scale(e.target())
                .ok_or("unsupported dose target")?;
            if !scale.is_finite() || scale < 0.0 || !(amount * scale).is_finite() {
                return Err("invalid dose scale or overflow".into());
            }
            if let Event::Infusion {
                time,
                amount,
                duration,
                ..
            } = e
            {
                if !duration.is_finite()
                    || *duration <= 0.0
                    || time + duration > self.end
                    || time + duration <= *time
                    || !(amount / duration * scale).is_finite()
                {
                    return Err("invalid infusion duration or rate".into());
                }
            }
        }
        Ok(())
    }
    fn boundaries(&self) -> Vec<f64> {
        let mut times = vec![self.start, self.end];
        for e in &self.events {
            times.push(e.time());
            if let Event::Infusion { time, duration, .. } = e {
                times.push(time + duration);
            }
        }
        sorted_unique(&mut times);
        times
    }
    fn has_event(&self, t: f64) -> bool {
        self.events.iter().any(|e| {
            e.time() == t || matches!(e, Event::Infusion{time,duration,..} if time+duration == t)
        })
    }
}

fn simulation_schedule<M: Model>(model: &M, protocol: &Protocol) -> (Vec<f64>, Vec<f64>) {
    let mut boundaries = protocol.boundaries();
    boundaries.extend_from_slice(model.discontinuities());
    sorted_unique(&mut boundaries);
    let mut samples = protocol.samples.clone();
    sorted_unique(&mut samples);
    (boundaries, samples)
}

/// Stored trajectory coordinates, without evaluating model callbacks or outputs.
fn simulation_rows<M: Model>(model: &M, protocol: &Protocol) -> Vec<(f64, &'static str)> {
    let (boundaries, samples) = simulation_schedule(model, protocol);
    let mut rows = Vec::new();
    for (index, &time) in boundaries.iter().enumerate() {
        if protocol.has_event(time) || model.discontinuities().contains(&time) {
            rows.push((time, "pre"));
        }
        rows.push((time, "post"));
        if let Some(&end) = boundaries.get(index + 1) {
            rows.extend(
                samples
                    .iter()
                    .copied()
                    .filter(|t| *t > time && *t < end)
                    .map(|t| (t, "post")),
            );
        }
    }
    rows
}

/// Segment only at true dosing discontinuities, never at ordinary sample times.
/// Every boundary has explicit pre/post output; observations elsewhere are post.
pub fn simulate<M: Model>(model: &M, protocol: &Protocol) -> Result<Vec<Sample>, Error> {
    simulate_with_budgets(model, protocol, Budgets::default())
}

pub fn simulate_with_budgets<M: Model>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
) -> Result<Vec<Sample>, Error> {
    simulate_with_absolute_tolerances(model, protocol, budgets, None)
}

pub(crate) fn simulate_with_absolute_tolerances<M: Model>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
    absolute_tolerances: Option<&[f64]>,
) -> Result<Vec<Sample>, Error> {
    simulate_with_solver(
        model,
        protocol,
        budgets,
        absolute_tolerances,
        Solver::DiffsolBdf,
    )
}

pub(crate) fn simulate_with_solver<M: Model>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
    absolute_tolerances: Option<&[f64]>,
    solver_choice: Solver,
) -> Result<Vec<Sample>, Error> {
    simulate_with_solver_accuracy(
        model,
        protocol,
        budgets,
        absolute_tolerances,
        solver_choice,
        false,
    )
}

/// Require bounded convergence of sampled states with the same DiffSol method.
/// Local tolerances are refined within each event interval, sharing the supplied
/// callback budget. This is convergence checking, not a universal global bound.
pub fn simulate_with_sampled_accuracy<M: Model>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
    absolute_tolerances: Option<&[f64]>,
    solver_choice: Solver,
) -> Result<Vec<Sample>, Error> {
    simulate_with_solver_accuracy(
        model,
        protocol,
        budgets,
        absolute_tolerances,
        solver_choice,
        true,
    )
}

pub(crate) fn simulate_with_solver_accuracy<M: Model>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
    absolute_tolerances: Option<&[f64]>,
    solver_choice: Solver,
    sampled_accuracy: bool,
) -> Result<Vec<Sample>, Error> {
    let pattern = model.jacobian_sparsity();
    if pattern
        .as_ref()
        .is_some_and(|p| p.iter().any(|&(r, c)| r >= 63 || c >= 63))
    {
        simulate_matrix::<M, diffsol::FaerSparseMat<f64>, diffsol::FaerSparseLU<f64>>(
            model,
            protocol,
            budgets,
            absolute_tolerances,
            solver_choice,
            sampled_accuracy,
        )
    } else {
        simulate_matrix::<M, NalgebraMat<f64>, AdaptiveLU>(
            model,
            protocol,
            budgets,
            absolute_tolerances,
            solver_choice,
            sampled_accuracy,
        )
    }
}
/// Static driver admission shared by scientific preparation and integration.
/// No RHS, Jacobian, interval propagation or trajectory output is evaluated.
fn prepare_simulation<M: Model>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
    absolute_tolerances: Option<&[f64]>,
) -> Result<Vec<f64>, Error> {
    let event_count = protocol
        .events
        .len()
        .checked_add(model.discontinuities().len())
        .ok_or_else(|| Error::new(ErrorCode::WorkBudget, "event count overflow"))?;
    if event_count > budgets.events {
        return Err(Error::new(
            ErrorCode::WorkBudget,
            "event count exceeds budget",
        ));
    }
    if model
        .discontinuities()
        .iter()
        .any(|t| !t.is_finite() || *t < protocol.start || *t > protocol.end)
        || model.discontinuities().windows(2).any(|w| w[0] >= w[1])
    {
        return Err("invalid binding discontinuities".into());
    }
    protocol.validate(model)?;
    let state = model.initial();
    let n = state.len();
    let scalar = [protocol.atol];
    let tolerances = absolute_tolerances.unwrap_or(&scalar);
    if (absolute_tolerances.is_some() && tolerances.len() != n)
        || tolerances.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err("absolute tolerances must be finite, positive and match every state".into());
    }
    // Each event adds at most two boundaries, with pre/post rows at each.
    let row_limit = event_count
        .checked_mul(4)
        .and_then(|v| v.checked_add(protocol.samples.len()))
        .and_then(|v| v.checked_add(2))
        .and_then(|v| v.checked_mul(n));
    if row_limit.is_none_or(|v| v > budgets.output_values) {
        return Err(Error::new(
            ErrorCode::OutputBudget,
            "requested output exceeds budget",
        ));
    }
    Ok(state)
}
fn simulate_matrix<M, Mat, LS>(
    model: &M,
    protocol: &Protocol,
    budgets: Budgets,
    absolute_tolerances: Option<&[f64]>,
    solver_choice: Solver,
    sampled_accuracy: bool,
) -> Result<Vec<Sample>, Error>
where
    M: Model,
    Mat: diffsol::Matrix<T = f64>,
    LS: diffsol::LinearSolver<Mat>,
    Mat::V: diffsol::DefaultDenseMatrix<T = f64, C = Mat::C>,
    for<'b> &'b Mat::V: diffsol::VectorRef<Mat::V>,
    for<'b> &'b Mat: diffsol::matrix::MatrixRef<Mat>,
{
    use diffsol::Vector;
    let mut state = prepare_simulation(model, protocol, budgets, absolute_tolerances)?;
    let n = state.len();
    let scalar = [protocol.atol];
    let tolerances = absolute_tolerances.unwrap_or(&scalar);
    let callbacks = Cell::new(0u64);
    let first_error = RefCell::new(None::<Error>);
    let recover_trial_domain = Cell::new(false);
    let trial_origin = Cell::new(protocol.start);
    let last_trial_error = RefCell::new(None::<Error>);
    let guarded =
        |time: f64, out: &mut [f64], evaluate: &dyn Fn(&mut [f64]) -> Result<(), Error>| {
            let count = callbacks.get().saturating_add(1);
            callbacks.set(count);
            if first_error.borrow().is_none() {
                let result = if count > budgets.solver_callbacks {
                    Err(Error::new(
                        ErrorCode::WorkBudget,
                        "solver callback budget exhausted",
                    ))
                } else {
                    evaluate(out).and_then(|()| {
                        if out.iter().any(|v| !v.is_finite()) {
                            Err(Error::new(ErrorCode::Domain, "non-finite model evaluation"))
                        } else {
                            Ok(())
                        }
                    })
                };
                if let Err(mut error) = result {
                    error.time = Some(time);
                    if recover_trial_domain.get()
                        && time > trial_origin.get()
                        && error.code == ErrorCode::Domain
                    {
                        // Report a non-finite trial to the common solver. It must
                        // shrink and retry before replacing the accepted state.
                        *last_trial_error.borrow_mut() = Some(error);
                        out.fill(f64::NAN);
                    } else {
                        *first_error.borrow_mut() = Some(error);
                    }
                }
            }
            if first_error.borrow().is_some() {
                out.fill(f64::NAN);
            }
        };
    let solver_error = |error: diffsol::DiffsolError| {
        first_error
            .borrow()
            .clone()
            .unwrap_or_else(|| Error::new(ErrorCode::Solver, error.to_string()))
    };
    let (boundaries, samples) = simulation_schedule(model, protocol);
    let mut output = Vec::new();
    let mut stopped_infusions = std::collections::BTreeSet::new();
    for (index, &t) in boundaries.iter().enumerate() {
        model.check_state(t, &state)?;
        if protocol.has_event(t) || model.discontinuities().contains(&t) {
            output.push(Sample {
                time: t,
                side: "pre".into(),
                values: state.clone(),
            });
        }
        model.enter_boundary(t)?;
        let mut resets: Vec<_> = protocol
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Reset {
                    time,
                    target,
                    value,
                    order,
                    active_inputs,
                } if *time == t => Some((*order, *target, *value, *active_inputs)),
                _ => None,
            })
            .collect();
        resets.sort_by_key(|(order, ..)| *order);
        for (_, target, value, active_inputs) in resets {
            model.apply_reset(t, target, value, &mut state)?;
            model.check_state(t, &state)?;
            if active_inputs == qspquest_core::ActiveInputs::StopTarget {
                for (id, event) in protocol.events.iter().enumerate() {
                    if matches!(event,Event::Infusion{time,target:input_target,duration,..} if *input_target==target&&*time<t&&t<*time+*duration)
                    {
                        stopped_infusions.insert(id);
                    }
                }
            }
        }
        for e in &protocol.events {
            if let Event::Bolus {
                time,
                target,
                amount,
            } = e
            {
                if *time == t {
                    model.apply_bolus(t, *target, *amount, &mut state)?;
                    model.check_state(t, &state)?;
                }
            }
        }
        if state.iter().any(|x| !x.is_finite()) {
            return Err("combined bolus overflow".into());
        }
        output.push(Sample {
            time: t,
            side: "post".into(),
            values: state.clone(),
        });
        if index + 1 == boundaries.len() {
            break;
        }
        let end = boundaries[index + 1];
        let mut rates = vec![0.0; n];
        for (id, e) in protocol.events.iter().enumerate() {
            if stopped_infusions.contains(&id) {
                continue;
            }
            if let Event::Infusion {
                time,
                target,
                amount,
                duration,
            } = e
            {
                if *time <= t && t < time + duration {
                    model.accumulate_infusion_rate(t, *target, amount / duration, &mut rates)?;
                }
            }
        }
        if rates.iter().chain(state.iter()).any(|x| !x.is_finite()) {
            return Err("combined event overflow".into());
        }
        let mut eval: Vec<f64> = samples
            .iter()
            .copied()
            .filter(|x| *x > t && *x < end)
            .collect();
        eval.push(end);
        if let Some(propagator) = model.interval_propagator() {
            // Every sample is evaluated from the same interval origin; adding
            // observations must not alter subsequent trajectory values.
            let initial = state.clone();
            for time in eval {
                let count = callbacks.get().saturating_add(1);
                callbacks.set(count);
                if count > budgets.solver_callbacks {
                    let mut error = Error::new(
                        ErrorCode::WorkBudget,
                        "analytic propagation budget exhausted",
                    );
                    error.time = Some(time);
                    return Err(error);
                }
                let values =
                    propagator
                        .propagate(&initial, &rates, time - t)
                        .map_err(|mut error| {
                            error.time = Some(time);
                            error
                        })?;
                if values.len() != n || values.iter().any(|v| !v.is_finite()) {
                    let mut error =
                        Error::new(ErrorCode::Domain, "invalid analytic propagation result");
                    error.time = Some(time);
                    return Err(error);
                }
                model.check_state(time, &values)?;
                if time == end {
                    state = values;
                } else {
                    output.push(Sample {
                        time,
                        side: "post".into(),
                        values,
                    });
                }
            }
            continue;
        }
        let initial = state.clone();
        // Integrate elapsed interval time so tiny accepted steps can advance
        // even when the scientific clock has a large positive/negative origin.
        // Model callbacks and all reported observations retain absolute time.
        let interval_duration = end - t;
        if !interval_duration.is_finite() || interval_duration <= 0.0 {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "invalid interval duration",
            ));
        }
        let is_rosenbrock = matches!(
            solver_choice,
            Solver::DiffsolRosenbrock23 | Solver::DiffsolRodas5p
        );
        let stop_is_discontinuity =
            protocol.has_event(end) || model.rhs_jump_times().contains(&end);
        let previous_rhs_time = Cell::new(None::<(f64, f64)>);
        let mut problem = OdeBuilder::<Mat>::new()
            .t0(0.0)
            .rtol(protocol.rtol)
            .atol(tolerances.iter().copied())
            .rhs_implicit(
                |x, _p, elapsed, y| {
                    let mut time = t + elapsed;
                    if is_rosenbrock && first_error.borrow().is_none() {
                        let collapsed_probe = previous_rhs_time
                            .get()
                            .is_some_and(|(previous_elapsed, previous_absolute)| {
                                let local_gap = (previous_elapsed - elapsed).abs();
                                let ulp_margin = 8.0
                                    * f64::EPSILON
                                    * (1.0 + previous_elapsed.abs().max(elapsed.abs()));
                                local_gap > ulp_margin && previous_absolute == time
                            });
                        let rounded_into_jump =
                            stop_is_discontinuity && elapsed < interval_duration && time >= end;
                        if rounded_into_jump {
                            let incoming = incoming_f64(end);
                            let resolution_limit =
                                f64::EPSILON.sqrt() * (1.0 + interval_duration);
                            if incoming > t && end - incoming <= resolution_limit {
                                time = incoming;
                            }
                        }
                        let unresolved_jump = rounded_into_jump && time >= end;
                        // Interval rates and binding state stay fixed until enter_boundary.
                        // An autonomous RHS does not inspect the rounded clock probe.
                        if (collapsed_probe || unresolved_jump) && model.rhs_depends_on_time() {
                            let mut error = Error::new(
                                ErrorCode::Unsupported,
                                "Rosenbrock time probes are not representable on the absolute model clock",
                            );
                            error.time = Some(time);
                            *first_error.borrow_mut() = Some(error);
                        }
                        previous_rhs_time.set(Some((elapsed, time)));
                    }
                    guarded(time, y, &|out| {
                        model.rhs(time, x, &rates, out)
                    })
                },
                |x, _p, elapsed, v, y| {
                    let time = t + elapsed;
                    guarded(time, y, &|out| {
                        model.jac_mul(time, x, &rates, v, out)
                    })
                },
            )
            .rhs_sparsity(model.jacobian_sparsity())
            .init(
                move |_p, _time, y| {
                    for i in 0..n {
                        y[i] = initial[i];
                    }
                },
                n,
            )
            .build()
            .map_err(&solver_error)?;
        if let Some(error) = first_error.borrow().clone() {
            return Err(error);
        }
        // All methods share the same event driver, dense observation handling,
        // callback budget, and validation of physical states. Only the solver
        // construction and nonlinear failure control differ.
        macro_rules! integrate_interval {
            ($constructor:expr, $configure:expr, $stop:expr) => {{
                // Initial-step estimation also evaluates an unaccepted trial.
                // Failures at the interval origin remain permanent errors.
                trial_origin.set(t);
                recover_trial_domain.set(is_rosenbrock);
                let constructor_result = $constructor;
                recover_trial_domain.set(false);
                let mut solver = constructor_result.map_err(&solver_error)?;
                if let Some(error) = first_error.borrow().clone() {
                    return Err(error);
                }
                // Permit any representable step; progress and callback guards
                // below bound the solve when tolerances are very small.
                solver.config_mut().minimum_timestep = 0.0;
                $configure(&mut solver);
                $stop(&mut solver).map_err(|source| {
                    let mut error = solver_error(source);
                    if error.time.is_none() {
                        error.time = Some(t);
                    }
                    error
                })?;
                let mut column = 0;
                while column < eval.len() {
                    let previous_time = solver.state().t;
                    *last_trial_error.borrow_mut() = None;
                    recover_trial_domain.set(is_rosenbrock);
                    let step_result = solver.step();
                    recover_trial_domain.set(false);
                    step_result.map_err(|source| {
                        let mut error = first_error.borrow().clone()
                            .or_else(|| last_trial_error.borrow().clone())
                            .unwrap_or_else(|| solver_error(source));
                        if error.time.is_none() {
                            error.time = Some(t + previous_time);
                        }
                        error
                    })?;
                    if let Some(error) = first_error.borrow().clone() {
                        return Err(error);
                    }
                    let reached = solver.state().t;
                    model.check_state(t + reached, &solver.state().y.clone_as_vec())?;
                    if !reached.is_finite() || reached <= previous_time {
                        let mut error = Error::new(
                            ErrorCode::Solver,
                            format!("solver made no forward progress: elapsed_previous={previous_time}, elapsed_reached={reached}, next_step={}, interval_origin={t}, interval_end={end}", solver.state().h),
                        );
                        error.time = Some(t + previous_time);
                        return Err(error);
                    }
                    while column < eval.len() && eval[column] - t <= reached {
                        let time = eval[column];
                        let values = solver
                            .interpolate(time - t)
                            .map_err(&solver_error)?
                            .clone_as_vec();
                        model.check_state(time, &values)?;
                        column += 1;
                        if values.iter().any(|x| !x.is_finite()) {
                            return Err("nonfinite solve".into());
                        }
                        if time == end {
                            state = values;
                        } else {
                            output.push(Sample {
                                time,
                                side: "post".into(),
                                values,
                            });
                        }
                    }
                }
            }};
        }
        // Reuse the shared convergence policy for every numerical method.
        // Restart only this smooth interval; dose/reset events are applied once
        // by the outer driver. Callback counts and errors are never reset.
        let interval_initial = state.clone();
        let original_atol = problem.atol.clone();
        let output_start = output.len();
        let mut solve_interval = |factor| {
            state.clone_from(&interval_initial);
            output.truncate(output_start);
            previous_rhs_time.set(None);
            problem.rtol = protocol.rtol * factor;
            problem.atol = original_atol.clone();
            problem.atol *= diffsol::scale(factor);
            match solver_choice {
                Solver::DiffsolBdf => integrate_interval!(
                    problem.bdf::<LS>(),
                    |solver: &mut diffsol::Bdf<_, _>| {
                        solver.config_mut().maximum_newton_fails =
                            budgets.solver_callbacks.min(2_000) as usize;
                    },
                    |solver: &mut diffsol::Bdf<_, _>| solver.set_stop_time(interval_duration)
                ),
                Solver::DiffsolEsdirk34 => integrate_interval!(
                    problem.esdirk34::<LS>(),
                    |solver: &mut diffsol::Sdirk<_, _, _>| {
                        solver.config_mut().maximum_newton_fails =
                            budgets.solver_callbacks.min(2_000) as usize;
                    },
                    |solver: &mut diffsol::Sdirk<_, _, _>| solver.set_stop_time(interval_duration)
                ),
                Solver::DiffsolTrBdf2 => integrate_interval!(
                    problem.tr_bdf2::<LS>(),
                    |solver: &mut diffsol::Sdirk<_, _, _>| {
                        solver.config_mut().maximum_newton_fails =
                            budgets.solver_callbacks.min(2_000) as usize;
                    },
                    |solver: &mut diffsol::Sdirk<_, _, _>| solver.set_stop_time(interval_duration)
                ),
                Solver::DiffsolTsit45 => integrate_interval!(
                    problem.tsit45(),
                    |_solver: &mut _| {},
                    |solver: &mut diffsol::ExplicitRk<_, _>| solver
                        .set_stop_time(interval_duration)
                ),
                Solver::DiffsolRosenbrock23 => integrate_interval!(
                    problem.rosenbrock23::<LS>(),
                    |solver: &mut diffsol::Rosenbrock<_, _>| solver
                        .set_time_derivative_within_step(true),
                    |solver: &mut diffsol::Rosenbrock23<_, _>| {
                        if stop_is_discontinuity {
                            solver.set_discontinuity_stop_time(interval_duration)
                        } else {
                            solver.set_stop_time(interval_duration)
                        }
                    }
                ),
                Solver::DiffsolRodas5p => integrate_interval!(
                    problem.rodas5p::<LS>(),
                    |solver: &mut diffsol::Rosenbrock<_, _>| solver
                        .set_time_derivative_within_step(true),
                    |solver: &mut diffsol::Rodas5P<_, _>| {
                        if stop_is_discontinuity {
                            solver.set_discontinuity_stop_time(interval_duration)
                        } else {
                            solver.set_stop_time(interval_duration)
                        }
                    }
                ),
                Solver::AnalyticLinearPk => {
                    return Err(Error::new(
                        ErrorCode::Unsupported,
                        "analytic_linear_pk requires a linear_pk declaration",
                    ));
                }
            }
            Ok(output[output_start..]
                .iter()
                .map(|row| row.values.clone())
                .chain(std::iter::once(state.clone()))
                .collect())
        };
        if sampled_accuracy || matches!(solver_choice, Solver::DiffsolTrBdf2) {
            diffsol::refine_sampled_trajectory(
                protocol.rtol,
                tolerances,
                if sampled_accuracy { 8 } else { 6 },
                if sampled_accuracy { 0.25 } else { 1.0 },
                &mut solve_interval,
                |source| {
                    let mut error = solver_error(source);
                    if error.time.is_none() {
                        error.time = Some(end);
                    }
                    error
                },
            )?;
        } else {
            solve_interval(1.0)?;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod rosenbrock_integration_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct TrialDomain {
        failures: Cell<usize>,
        persistent: bool,
    }
    impl Model for TrialDomain {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn initial(&self) -> Vec<f64> {
            vec![1.0]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            None
        }
        fn rhs_depends_on_time(&self) -> bool {
            false
        }
        fn rhs(&self, time: f64, _: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
            // A single failed stage is a deterministic rejection oracle. It is
            // not a model benchmark; the endpoint after retry remains 1-t.
            if time > 0.0 && (self.persistent || self.failures.get() == 0) {
                self.failures.set(self.failures.get() + 1);
                return Err(Error::new(ErrorCode::Domain, "non-finite trial evaluation"));
            }
            out[0] = -1.0;
            Ok(())
        }
        fn jac_mul(
            &self,
            _: f64,
            _: &[f64],
            _: &[f64],
            _: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            out[0] = 0.0;
            Ok(())
        }
    }

    #[test]
    fn rosenbrock_trial_domain_errors_retry_and_persistent_errors_fail() {
        for solver in [Solver::DiffsolRosenbrock23, Solver::DiffsolRodas5p] {
            let model = TrialDomain {
                failures: Cell::new(0),
                persistent: false,
            };
            let rows = simulate_with_solver(
                &model,
                &protocol(0.0, 0.1, vec![0.05, 0.1]),
                Budgets::default(),
                None,
                solver,
            )
            .unwrap();
            assert_eq!(model.failures.get(), 1);
            assert!((rows.last().unwrap().values[0] - 0.9).abs() < 1e-8);
            let model = TrialDomain {
                failures: Cell::new(0),
                persistent: true,
            };
            let error = simulate_with_solver(
                &model,
                &protocol(0.0, 0.1, vec![0.1]),
                Budgets::default(),
                None,
                solver,
            )
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::Domain);
        }
    }

    #[test]
    fn rosenbrock_domain_retries_preserve_callback_budget() {
        let model = TrialDomain {
            failures: Cell::new(0),
            persistent: true,
        };
        let error = simulate_with_solver(
            &model,
            &protocol(0.0, 0.1, vec![0.1]),
            Budgets {
                solver_callbacks: 30,
                ..Budgets::default()
            },
            None,
            Solver::DiffsolRodas5p,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkBudget);
    }

    struct TimeForcing {
        origin: f64,
    }

    impl Model for TimeForcing {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn initial(&self) -> Vec<f64> {
            vec![0.0]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            None
        }
        fn rhs(&self, time: f64, _: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
            out[0] = time - self.origin;
            Ok(())
        }
        fn jac_mul(
            &self,
            _: f64,
            _: &[f64],
            _: &[f64],
            _: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            out[0] = 0.0;
            Ok(())
        }
    }

    struct TimeJump;

    struct AutonomousInput;
    impl Model for AutonomousInput {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn initial(&self) -> Vec<f64> {
            vec![0.]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            Some(1.)
        }
        fn rhs_depends_on_time(&self) -> bool {
            false
        }
        fn rhs(&self, _: f64, _: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
            out[0] = rates[0];
            Ok(())
        }
        fn jac_mul(
            &self,
            _: f64,
            _: &[f64],
            _: &[f64],
            _: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            out[0] = 0.;
            Ok(())
        }
    }

    #[test]
    fn autonomous_infusion_keeps_observation_one_ulp_before_stop() {
        let start = 42.;
        let duration = 1. / 48.;
        let stop = start + duration;
        let near = incoming_f64(stop);
        for solver in [Solver::DiffsolRosenbrock23, Solver::DiffsolRodas5p] {
            let mut p = protocol(start, stop + 1., vec![near, stop, stop + 1.]);
            p.events.push(Event::Infusion {
                time: start,
                target: 0,
                amount: 1.,
                duration,
            });
            let rows = simulate_with_solver(&AutonomousInput, &p, Budgets::default(), None, solver)
                .unwrap();
            let observed = rows.iter().find(|row| row.time == near).unwrap();
            assert!((observed.values[0] - (near - start) / duration).abs() < 1e-10);
            assert!((rows.last().unwrap().values[0] - 1.).abs() < 1e-10);
            assert!(rows.iter().any(|row| row.time == stop));
        }
    }

    struct NonlinearDecay {
        jacobian_times: RefCell<Vec<f64>>,
    }

    impl Model for NonlinearDecay {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn initial(&self) -> Vec<f64> {
            vec![2.0]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            None
        }
        fn rhs(&self, _: f64, x: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
            out[0] = -x[0] * x[0];
            Ok(())
        }
        fn jac_mul(
            &self,
            time: f64,
            x: &[f64],
            _: &[f64],
            v: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            self.jacobian_times.borrow_mut().push(time);
            out[0] = -2.0 * x[0] * v[0];
            Ok(())
        }
    }

    impl Model for TimeJump {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn discontinuities(&self) -> &[f64] {
            &[1.0]
        }
        fn initial(&self) -> Vec<f64> {
            vec![0.0]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            Some(1.0)
        }
        fn rhs(&self, time: f64, _: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
            out[0] = if time < 1.0 { 0.0 } else { 1.0 };
            Ok(())
        }
        fn jac_mul(
            &self,
            _: f64,
            _: &[f64],
            _: &[f64],
            _: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            out[0] = 0.0;
            Ok(())
        }
    }

    struct EndpointProbe {
        saw_exact_end: Cell<bool>,
        checkpoint: bool,
    }

    impl Model for EndpointProbe {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn discontinuities(&self) -> &[f64] {
            if self.checkpoint {
                &[1.0]
            } else {
                &[]
            }
        }
        fn rhs_jump_times(&self) -> &[f64] {
            &[]
        }
        fn initial(&self) -> Vec<f64> {
            vec![0.0]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            None
        }
        fn rhs(&self, time: f64, _: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
            if time == 1.0 {
                self.saw_exact_end.set(true);
            }
            out[0] = 1.0;
            Ok(())
        }
        fn jac_mul(
            &self,
            _: f64,
            _: &[f64],
            _: &[f64],
            _: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            out[0] = 0.0;
            Ok(())
        }
    }

    fn protocol(start: f64, end: f64, samples: Vec<f64>) -> Protocol {
        Protocol {
            start,
            end,
            samples,
            events: vec![],
            rtol: 1e-8,
            atol: 1e-10,
        }
    }

    #[test]
    fn smooth_absolute_time_forcing_and_declared_jump() {
        for solver in [Solver::DiffsolRosenbrock23, Solver::DiffsolRodas5p] {
            let smooth = simulate_with_solver(
                &TimeForcing { origin: 2.0 },
                &protocol(2.0, 6.0, vec![2.0, 6.0]),
                Budgets::default(),
                None,
                solver,
            )
            .unwrap();
            assert!((smooth.last().unwrap().values[0] - 8.0).abs() < 1e-6);

            let covariate_only_jump = simulate_with_solver(
                &TimeJump,
                &protocol(0.0, 2.0, vec![1.0, 2.0]),
                Budgets::default(),
                None,
                solver,
            )
            .unwrap();
            let before_change = covariate_only_jump
                .iter()
                .find(|row| row.time == 1.0 && row.side == "pre")
                .unwrap();
            let after_change = covariate_only_jump
                .iter()
                .find(|row| row.time == 1.0 && row.side == "post")
                .unwrap();
            assert!(before_change.values[0].abs() < 1e-8);
            assert!(after_change.values[0].abs() < 1e-8);
            assert!((covariate_only_jump.last().unwrap().values[0] - 1.0).abs() < 1e-6);

            let jumped = simulate_with_solver(
                &TimeJump,
                &Protocol {
                    events: vec![Event::Bolus {
                        time: 1.0,
                        target: 0,
                        amount: 2.0,
                    }],
                    ..protocol(0.0, 2.0, vec![0.5, 1.0, 1.5, 2.0])
                },
                Budgets::default(),
                None,
                solver,
            )
            .unwrap();
            let pre = jumped
                .iter()
                .find(|row| row.time == 1.0 && row.side == "pre")
                .unwrap();
            let post = jumped
                .iter()
                .find(|row| row.time == 1.0 && row.side == "post")
                .unwrap();
            assert!(pre.values[0].abs() < 1e-8, "{solver:?}: {pre:?}");
            assert!((post.values[0] - 2.0).abs() < 1e-8, "{solver:?}: {post:?}");
            let final_amount = jumped.last().unwrap().values[0];
            assert!(
                (final_amount - 3.0).abs() < 1e-6,
                "{solver:?}: {final_amount}"
            );
        }
    }

    #[test]
    fn ordinary_final_target_evaluates_rhs_at_exact_endpoint() {
        for solver in [Solver::DiffsolRosenbrock23, Solver::DiffsolRodas5p] {
            for checkpoint in [false, true] {
                let model = EndpointProbe {
                    saw_exact_end: Cell::new(false),
                    checkpoint,
                };
                let rows = simulate_with_solver(
                    &model,
                    &protocol(0.0, 1.0, vec![1.0]),
                    Budgets::default(),
                    None,
                    solver,
                )
                .unwrap();
                assert!(
                    model.saw_exact_end.get(),
                    "{solver:?} skipped the smooth endpoint with checkpoint={checkpoint}"
                );
                assert!((rows.last().unwrap().values[0] - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn nonrepresentable_absolute_time_probes_fail_explicitly() {
        for origin in [-1e16, 1e16] {
            for solver in [Solver::DiffsolRosenbrock23, Solver::DiffsolRodas5p] {
                let result = simulate_with_solver(
                    &TimeForcing { origin },
                    &protocol(origin, origin + 4.0, vec![origin, origin + 4.0]),
                    Budgets::default(),
                    None,
                    solver,
                );
                let error = result.unwrap_err();
                assert_eq!(error.code, ErrorCode::Unsupported, "{solver:?}: {error}");
                assert!(error.message.contains("not representable"));
            }
        }
    }

    #[test]
    fn nonlinear_jacobian_is_refreshed_within_a_qspquest_interval() {
        for solver in [Solver::DiffsolRosenbrock23, Solver::DiffsolRodas5p] {
            let model = NonlinearDecay {
                jacobian_times: RefCell::new(Vec::new()),
            };
            let rows = simulate_with_solver(
                &model,
                &protocol(0.0, 1.0, vec![1.0]),
                Budgets::default(),
                None,
                solver,
            )
            .unwrap();
            let final_value = rows.last().unwrap().values[0];
            let exact = 2.0 / 3.0;
            assert!(
                (final_value - exact).abs() < 2e-5,
                "{solver:?}: {final_value}"
            );
            assert!(
                model
                    .jacobian_times
                    .borrow()
                    .iter()
                    .any(|time| *time > 0.0 && *time < 1.0),
                "{solver:?} reused its initial nonlinear Jacobian"
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/fixture_support/mod.rs"]
mod test_fixture;
