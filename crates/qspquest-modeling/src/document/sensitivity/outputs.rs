//! Observe augmented trajectories with the raw infusion inputs on each side.
use super::*;
use qspquest_core::{ActiveInputs, Event};
#[derive(Clone, Debug)]
pub struct SensitivitySample {
    pub time: f64,
    pub side: String,
    pub values: Vec<f64>,
    /// Selected parameter-major output derivatives.
    pub derivatives: Vec<Vec<f64>>,
}
pub(super) struct PreparedSensitivityProjection {
    pub(super) selected: Vec<(usize, usize, String)>,
}
impl PreparedSensitivityProjection {
    pub(super) fn len(&self) -> usize {
        self.selected.len()
    }
}

struct InputSchedule {
    changes: Vec<(f64, usize, Option<(usize, f64)>)>,
    cursor: usize,
    active: BTreeMap<usize, (usize, f64)>,
    rates: Vec<f64>,
}
impl InputSchedule {
    // Call only after protocol validation through the simulation driver.
    fn new(protocol: &crate::Protocol, n: usize) -> Self {
        let mut resets = vec![Vec::new(); n];
        for event in &protocol.events {
            if let Event::Reset {
                time,
                target,
                active_inputs: ActiveInputs::StopTarget,
                ..
            } = event
            {
                resets[*target].push(*time);
            }
        }
        for times in &mut resets {
            times.sort_by(f64::total_cmp);
        }
        let mut changes = Vec::new();
        for (id, event) in protocol.events.iter().enumerate() {
            if let Event::Infusion {
                time,
                target,
                amount,
                duration,
            } = event
            {
                let next = resets[*target].partition_point(|t| *t <= *time);
                let stop = resets[*target]
                    .get(next)
                    .copied()
                    .unwrap_or(f64::INFINITY)
                    .min(time + duration);
                changes.push((*time, id, Some((*target, amount / duration))));
                changes.push((stop, id, None));
            }
        }
        changes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Self {
            changes,
            cursor: 0,
            active: BTreeMap::new(),
            rates: vec![0.; n],
        }
    }
    fn at(&mut self, time: f64, side: &str) -> Result<&[f64], Error> {
        let mut changed = false;
        while let Some((t, id, value)) = self.changes.get(self.cursor) {
            if *t > time || (*t == time && side == "pre") {
                break;
            }
            match value {
                Some(value) => {
                    self.active.insert(*id, *value);
                }
                None => {
                    self.active.remove(id);
                }
            }
            self.cursor += 1;
            changed = true;
        }
        if changed {
            self.rates.fill(0.);
            // Stable source-event order matches the driver's accumulation order.
            for (target, rate) in self.active.values() {
                self.rates[*target] += rate;
            }
            if self.rates.iter().any(|r| !r.is_finite()) {
                return Err(invalid("raw infusion input overflow"));
            }
        }
        Ok(&self.rates)
    }
}
impl BoundSensitivityDocument {
    /// Low-level fixed protocol. Times/amounts and scalar tolerances are numeric
    /// in declared model/augmented-state units, as for the ordinary Model API.
    pub fn simulate_outputs(
        &self,
        protocol: &crate::Protocol,
        budgets: crate::Budgets,
    ) -> Result<Vec<SensitivitySample>, Error> {
        crate::prepare_simulation(self, protocol, budgets, None)?;
        let coordinates = crate::simulation_rows(self, protocol);
        let projection = self.prepare_projection(protocol, &coordinates, None, budgets)?;
        let rows = crate::simulate_with_budgets(self, protocol, budgets)?;
        self.project_prepared_rows(protocol, rows, projection, &[])
    }
    pub(super) fn prepare_projection(
        &self,
        protocol: &crate::Protocol,
        coordinates: &[(f64, &str)],
        observations: Option<&[qspquest_core::regimen::ObservationPoint]>,
        budgets: crate::Budgets,
    ) -> Result<PreparedSensitivityProjection, Error> {
        let mut selected = Vec::new();
        if let Some(points) = observations {
            for (destination, point) in points.iter().enumerate() {
                let time = quantity(&point.time)?.in_unit(self.compiled.primal.model.time_unit)?;
                let side = match point.side {
                    qspquest_core::regimen::ObservationSide::Pre => "pre",
                    qspquest_core::regimen::ObservationSide::Post => "post",
                };
                let first = coordinates.partition_point(|(t, _)| *t < time);
                let mut matching =
                    (first..coordinates.len()).take_while(|i| coordinates[*i].0 == time);
                let index = matching
                    .clone()
                    .find(|i| coordinates[*i].1 == side)
                    .or_else(|| {
                        if !protocol.has_event(time) && !self.checkpoints.contains(&time) {
                            matching.next()
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| invalid("requested sensitivity observation was not produced"))?;
                selected.push((index, destination, side.to_string()));
            }
        } else {
            selected.extend(
                coordinates
                    .iter()
                    .enumerate()
                    .map(|(i, (_, side))| (i, i, side.to_string())),
            );
        }
        let output_width = self
            .compiled
            .primal
            .outputs
            .len()
            .checked_mul(self.compiled.parameters().len() + 1)
            .ok_or_else(|| invalid("sensitivity output width overflow"))?;
        if coordinates
            .len()
            .checked_mul(self.compiled.forward.augmented_state_units.len())
            .and_then(|n| {
                selected
                    .len()
                    .checked_mul(output_width)
                    .and_then(|m| n.checked_add(m))
            })
            .is_none_or(|n| n > budgets.output_values)
        {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "augmented trajectory and sensitivity outputs exceed budget",
            ));
        }
        Ok(PreparedSensitivityProjection { selected })
    }
    pub(super) fn project_prepared_rows(
        &self,
        protocol: &crate::Protocol,
        rows: Vec<qspquest_core::Sample>,
        projection: PreparedSensitivityProjection,
        bindings: &[(f64, BoundSensitivityDocument)],
    ) -> Result<Vec<SensitivitySample>, Error> {
        let mut selected = projection.selected;
        let mut output = vec![None; selected.len()];
        selected.sort_by_key(|(index, _, _)| *index);
        let mut inputs = InputSchedule::new(protocol, self.compiled.primal.state_count);
        for (index, destination, side) in selected {
            let row = &rows[index];
            let index = bindings.partition_point(|(time, _)| {
                *time < row.time || (*time == row.time && row.side == "post")
            });
            let bound = if index == 0 {
                self
            } else {
                &bindings[index - 1].1
            };
            let values = bound.outputs(row.time, &row.values, inputs.at(row.time, &row.side)?)?;
            output[destination] = Some(SensitivitySample {
                time: row.time,
                side,
                values: values.values,
                derivatives: values.derivatives,
            });
        }
        Ok(output
            .into_iter()
            .map(|x| x.expect("every selected observation filled"))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_sides_preserve_overlaps_and_reset_cancellation() {
        let infusion = |time, target, amount, duration| Event::Infusion {
            time,
            target,
            amount,
            duration,
        };
        let protocol = crate::Protocol {
            start: 0.,
            end: 10.,
            samples: vec![],
            events: vec![
                infusion(0., 0, 20., 10.),
                infusion(1., 0, 12., 6.),
                infusion(0., 1, 30., 10.),
                Event::Reset {
                    time: 3.,
                    target: 0,
                    value: 0.,
                    order: 0,
                    active_inputs: ActiveInputs::StopTarget,
                },
                infusion(3., 0, 10., 2.),
                Event::Reset {
                    time: 4.,
                    target: 1,
                    value: 0.,
                    order: 0,
                    active_inputs: ActiveInputs::Continue,
                },
            ],
            rtol: 1e-8,
            atol: 1e-10,
        };
        let mut s = InputSchedule::new(&protocol, 2);
        for (t, side, expected) in [
            (0., "pre", [0., 0.]),
            (0., "post", [2., 3.]),
            (1., "pre", [2., 3.]),
            (1., "post", [4., 3.]),
            (3., "pre", [4., 3.]),
            (3., "post", [5., 3.]),
            (4., "post", [5., 3.]),
            (5., "pre", [5., 3.]),
            (5., "post", [0., 3.]),
            (7., "post", [0., 3.]),
            (10., "pre", [0., 3.]),
            (10., "post", [0., 0.]),
        ] {
            assert_eq!(s.at(t, side).unwrap(), expected);
        }
    }
}
