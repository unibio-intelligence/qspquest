//! Bounded cycle iteration. Time is interpreted as phase within the repeated cycle.
//! Convergence from a specified seed does not prove uniqueness or stability.
use crate::{Budgets, Error, ErrorCode, Event, Model, Protocol, ProtocolExt};
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
/// Fold periodic input carryover into at most three segments per administration.
/// Dose scaling must remain constant over the cycle; all states retain model units.
pub fn normalize_periodic_cycle<M: Model>(
    model: &M,
    cycle: &Protocol,
    budgets: Budgets,
) -> Result<Protocol, Error> {
    let period = cycle.end - cycle.start;
    if !period.is_finite() || period <= 0. || !cycle.start.is_finite() || !cycle.end.is_finite() {
        return Err(invalid(
            "periodic cycle must have finite increasing endpoints",
        ));
    }
    if cycle.events.len() > budgets.events {
        return Err(Error::new(
            ErrorCode::WorkBudget,
            "periodic input event budget exceeded",
        ));
    }
    let mut normalized = cycle.clone();
    normalized.events.clear();
    for event in &cycle.events {
        if !event.time().is_finite() || event.time() < cycle.start || event.time() >= cycle.end {
            return Err(invalid("periodic events must lie in the half-open cycle"));
        }
        if let Event::Infusion {
            time,
            target,
            amount,
            duration,
        } = event
        {
            if !duration.is_finite()
                || *duration <= 0.
                || !amount.is_finite()
                || *amount < 0.
                || !(*time + *duration).is_finite()
                || *time + *duration <= *time
            {
                return Err(invalid("periodic infusion must have nonnegative finite amount and a finite representable positive duration"));
            }
            let mut stop = *time + *duration;
            let mut has_stop_reset = false;
            for reset in &cycle.events {
                if let Event::Reset {
                    time: reset_time,
                    target: reset_target,
                    active_inputs: qspquest_core::ActiveInputs::StopTarget,
                    ..
                } = reset
                {
                    if reset_target == target {
                        has_stop_reset = true;
                        let next = if *reset_time > *time {
                            *reset_time
                        } else {
                            *reset_time + period
                        };
                        if next > *time && next < stop {
                            stop = next;
                        }
                    }
                }
            }
            let rate = *amount / *duration;
            let mut remaining = stop - *time;
            if !has_stop_reset {
                let complete = (remaining / period).floor();
                if !complete.is_finite() {
                    return Err(invalid("periodic overlap count overflow"));
                }
                if complete >= 1. {
                    normalized.events.push(Event::Infusion {
                        time: cycle.start,
                        target: *target,
                        amount: rate * period * complete,
                        duration: period,
                    });
                    remaining %= period;
                }
            }
            if remaining > 0. {
                let head = remaining.min(cycle.end - *time);
                normalized.events.push(Event::Infusion {
                    time: *time,
                    target: *target,
                    amount: rate * head,
                    duration: head,
                });
                let tail = remaining - head;
                if tail > 0. {
                    normalized.events.push(Event::Infusion {
                        time: cycle.start,
                        target: *target,
                        amount: rate * tail,
                        duration: tail,
                    });
                }
            }
        } else {
            normalized.events.push(event.clone());
        }
        if normalized.events.len() > budgets.events {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "wrapped periodic event budget exceeded",
            ));
        }
    }
    normalized.validate(model)?;
    Ok(normalized)
}
#[derive(Clone, Debug)]
pub struct CycleIterationOptions {
    /// Positive absolute convergence tolerance for every state, in its model unit.
    pub absolute_tolerances: Vec<f64>,
    /// Optional per-state solver tolerances, distinct from convergence tolerances.
    pub integration_absolute_tolerances: Option<Vec<f64>>,
    pub relative_tolerance: f64,
    /// Includes the independent verification cycle; must be at least two.
    pub maximum_cycles: u32,
}
#[derive(Clone, Debug)]
pub struct CycleFixedPoint {
    /// State immediately before cycle-start events, from the last verification cycle.
    pub initial_states: Vec<f64>,
    pub residuals: Vec<f64>,
    pub maximum_scaled_residual: f64,
    pub cycles_completed: u32,
}
pub(crate) struct PreparedCycleIteration {
    pub(crate) seed: Vec<f64>,
    pub(crate) protocol: Protocol,
    pub(crate) options: CycleIterationOptions,
    pub(crate) partition: Budgets,
}
fn validate_cycle_iteration_options(
    seed: &[f64],
    options: &CycleIterationOptions,
) -> Result<(), Error> {
    if seed.is_empty()
        || seed.iter().any(|x| !x.is_finite())
        || options.absolute_tolerances.len() != seed.len()
        || options
            .absolute_tolerances
            .iter()
            .any(|x| !x.is_finite() || *x <= 0.)
        || !options.relative_tolerance.is_finite()
        || options.relative_tolerance < 0.
        || options.maximum_cycles < 2
    {
        return Err(invalid("cycle iteration requires finite initial states, per-state positive tolerances and at least two cycles"));
    }
    Ok(())
}
pub(crate) fn prepare_cycle_iteration<M: Model>(
    first: &M,
    seed: &[f64],
    cycle: &Protocol,
    options: &CycleIterationOptions,
    budgets: Budgets,
) -> Result<PreparedCycleIteration, Error> {
    validate_cycle_iteration_options(seed, options)?;
    if first.initial() != seed {
        return Err(invalid(
            "periodic binding did not preserve the supplied initial state",
        ));
    }
    let mut protocol = normalize_periodic_cycle(first, cycle, budgets)?;
    protocol.samples.clear();
    let passes = options.maximum_cycles;
    let partition = Budgets {
        solver_callbacks: budgets.solver_callbacks / u64::from(passes),
        events: budgets.events / passes as usize,
        output_values: budgets.output_values / passes as usize,
    };
    crate::prepare_simulation(
        first,
        &protocol,
        partition,
        options.integration_absolute_tolerances.as_deref(),
    )?;
    Ok(PreparedCycleIteration {
        seed: seed.to_vec(),
        protocol,
        options: options.clone(),
        partition,
    })
}
/// Iterate a phase-periodic model, binding a fresh model for each cycle.
/// The factory must preserve the supplied initial state exactly and keep all
/// other model parameters and semantics fixed. Two consecutive cycle residuals
/// must pass. A failed solve or exhausted iteration limit returns no partial state.
pub fn iterate_periodic_cycle<M: Model>(
    mut bind: impl FnMut(&[f64]) -> Result<M, Error>,
    seed: &[f64],
    cycle: &Protocol,
    options: &CycleIterationOptions,
    budgets: Budgets,
) -> Result<CycleFixedPoint, Error> {
    validate_cycle_iteration_options(seed, options)?;
    let first = bind(seed)?;
    let prepared = prepare_cycle_iteration(&first, seed, cycle, options, budgets)?;
    execute_prepared_cycle_iteration(bind, prepared)
}
pub(crate) fn execute_prepared_cycle_iteration<M: Model>(
    mut bind: impl FnMut(&[f64]) -> Result<M, Error>,
    prepared: PreparedCycleIteration,
) -> Result<CycleFixedPoint, Error> {
    let PreparedCycleIteration {
        seed,
        protocol,
        options,
        partition,
    } = prepared;
    let passes = options.maximum_cycles;
    let mut state = seed.to_vec();
    let mut prior_converged = false;
    for completed in 1..=passes {
        let model = bind(&state)?;
        if model.initial() != state {
            return Err(invalid(
                "periodic binding did not preserve the supplied initial state",
            ));
        }
        let rows = crate::simulate_with_absolute_tolerances(
            &model,
            &protocol,
            partition,
            options.integration_absolute_tolerances.as_deref(),
        )?;
        let terminal = &rows
            .last()
            .filter(|r| r.time == protocol.end)
            .ok_or_else(|| Error::new(ErrorCode::Solver, "missing periodic endpoint"))?
            .values;
        if terminal.len() != state.len() {
            return Err(invalid("periodic model changed state count"));
        }
        let residuals: Vec<_> = terminal.iter().zip(&state).map(|(a, b)| a - b).collect();
        let mut maximum = 0_f64;
        for (i, r) in residuals.iter().enumerate() {
            let scale = options.absolute_tolerances[i]
                + options.relative_tolerance * state[i].abs().max(terminal[i].abs());
            if !scale.is_finite() {
                return Err(invalid("periodic convergence scale overflow"));
            }
            let scaled = r.abs() / scale;
            if !scaled.is_finite() {
                return Err(Error::new(ErrorCode::Solver, "nonfinite periodic residual"));
            }
            maximum = maximum.max(scaled);
        }
        let converged = maximum <= 1.;
        if converged && prior_converged {
            return Ok(CycleFixedPoint {
                initial_states: state,
                residuals,
                maximum_scaled_residual: maximum,
                cycles_completed: completed,
            });
        }
        prior_converged = converged;
        state = terminal.clone();
    }
    Err(Error::new(
        ErrorCode::Solver,
        "periodic cycle iteration did not converge within maximum_cycles",
    ))
}
