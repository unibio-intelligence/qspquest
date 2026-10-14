"""Small convenience interfaces that retain explicit units and native contracts."""

from copy import deepcopy
from ._analysis import fail, finite, integer


def quantity(value, unit):
    """Build a finite native quantity; the model binder validates dimensions."""
    if not isinstance(unit, str) or not unit:
        fail("quantity requires a unit string")
    return {"value": finite(value, "quantity"), "unit": unit}


def bolus(
    target, amount, amount_unit, *, at=0.0, time_unit="h", bioavailability=1.0, lag=None
):
    """Build one bolus administration with an optional explicit lag."""
    if not isinstance(target, str) or not target or finite(amount, "amount") <= 0:
        fail("bolus requires a model target and positive amount")
    if (
        finite(at, "dose time") < 0
        or not 0 < finite(bioavailability, "bioavailability") <= 1
    ):
        fail("dose time must be nonnegative and bioavailability in (0, 1]")
    result = {
        "target": target,
        "amount": quantity(amount, amount_unit),
        "time": quantity(at, time_unit),
        "delivery": {"kind": "bolus"},
        "bioavailability": bioavailability,
    }
    if lag is not None:
        if finite(lag, "lag") < 0:
            fail("lag must be nonnegative")
        result["lag"] = quantity(lag, time_unit)
    return result


def infusion(
    target, amount, amount_unit, *, duration, at=0.0, time_unit="h", bioavailability=1.0
):
    """Build a fixed-duration infusion; the model must contain input forcing."""
    result = bolus(
        target,
        amount,
        amount_unit,
        at=at,
        time_unit=time_unit,
        bioavailability=bioavailability,
    )
    if finite(duration, "infusion duration") <= 0:
        fail("infusion duration must be positive")
    result["delivery"] = {
        "kind": "infusion",
        "span": {"kind": "duration", "duration": quantity(duration, time_unit)},
    }
    return result


def run_request(
    *,
    end,
    times,
    administrations=(),
    time_unit="h",
    side="post",
    solver="diffsol_bdf",
    parameters=None,
    rtol=1e-8,
    atol=1e-10,
    solver_callbacks=1000000,
    accuracy_control="local",
):
    """Build an explicit run request without changing solver or event semantics.

    Sampling times must be increasing; pass native requests directly when you
    need duplicate pre/post observations, resets, covariate changes, or repeats.
    """
    if finite(end, "end") <= 0 or side not in ("pre", "post"):
        fail("end must be positive and side must be pre or post")
    values = [finite(t, "sample time") for t in times]
    if (
        not values
        or any(t < 0 or t > end for t in values)
        or any(a >= b for a, b in zip(values, values[1:]))
    ):
        fail("times must be strictly increasing and within [0, end]")
    if finite(rtol, "rtol") <= 0 or finite(atol, "atol") <= 0:
        fail("tolerances must be positive")
    result = {
        "schema": "qspquest.run/v0.1",
        "solver": solver,
        "budgets": {
            "solver_callbacks": integer(
                solver_callbacks, "solver_callbacks", 1, 100000000
            ),
            "events": 10000,
            "output_values": 1000000,
        },
        "regimen": {
            "end": quantity(end, time_unit),
            "samples": [],
            "observations": [
                {"time": quantity(t, time_unit), "side": side} for t in values
            ],
            "administrations": deepcopy(list(administrations)),
            "rtol": rtol,
            "atol": atol,
        },
    }
    if accuracy_control not in ("local", "sampled"):
        fail("accuracy_control must be local or sampled")
    if accuracy_control != "local":
        result["accuracy_control"] = accuracy_control
    if parameters is not None:
        result["parameters"] = deepcopy(parameters)
    return result


def scientific_status(answer):
    """Explain result/error status and the limits on interpreting its contents."""
    code = getattr(answer, "code", None)
    if code is not None:
        meanings = {
            "cancelled": (
                "Cancelled",
                "The analysis stopped at a native-call boundary. Resume a verified checkpoint; no complete result was produced.",
            ),
            "unsupported": (
                "Unsupported request",
                "This method cannot evaluate the requested contract. Change the request or use an explicitly supported method.",
            ),
            "budget": (
                "Work limit reached",
                "Increase the declared budget after reviewing cost, or simplify the request. No complete result was produced.",
            ),
            "checkpoint": (
                "Checkpoint cannot be resumed",
                "Use the original inputs and environment, or start a new analysis with a new checkpoint path.",
            ),
        }
        title, message = meanings.get(code, ("Request failed", str(answer)))
        return {
            "status": code,
            "title": title,
            "message": message,
            "interpretation": "No complete scientific result.",
            "warnings": [],
        }
    status = answer.get("status", "complete")
    meanings = {
        "converged": (
            "Convergence checks passed",
            "The configured numerical checks passed. This does not prove a global optimum or biological validity.",
        ),
        "iteration_limit": (
            "More iterations may be needed",
            "The fit stopped before its convergence checks passed. Estimates and plots are exploratory.",
        ),
        "evaluation_limit": (
            "Simulation budget reached",
            "The fit exhausted its simulation budget. Any retained estimates are exploratory; final prediction rows may be absent.",
        ),
        "mixing_failed": (
            "Random effects did not mix reliably",
            "The population sampler failed its mixing check. Increase or revise sampling and examine identifiability before interpreting estimates.",
        ),
        "insufficient_successful_replicates": (
            "Too few successful bootstrap fits",
            "The requested success fraction was not reached. No bootstrap interval is available.",
        ),
        "incomplete": (
            "Some calculations did not complete",
            "Inspect individual failures. Do not treat missing intervals or grid points as successful results.",
        ),
        "complete": (
            "Calculation completed",
            "The requested calculation completed within its declared contract.",
        ),
    }
    title, message = meanings.get(
        status,
        (
            "Review this status",
            f"Inspect the recorded {status} status before interpreting the result.",
        ),
    )
    warnings = []
    if any(
        answer.get(k)
        for k in (
            "boundary_parameters",
            "boundary_residual_errors",
            "boundary_covariate_coefficients",
            "boundary_variances",
        )
    ):
        warnings.append(
            "An estimate is at a search bound; uncertainty and identifiability need review."
        )
    if answer.get("objective_kind") == "saem_complete_data_surrogate":
        warnings.append(
            "The population objective is a complete-data surrogate, not a marginal likelihood or model-comparison statistic."
        )
    if answer.get("schema") == "qspquest.vpc/v0.1" and answer.get(
        "lloq_coverage", 0
    ) < answer.get("total_observations", 0):
        warnings.append(
            "LLOQ coverage is incomplete; BLQ checks describe supplied limits only."
        )
    if answer.get("schema") == "qspquest.vpc/v0.1" and answer.get("failed_replicates", 0):
        warnings.append("Failed VPC replicates remain in the denominator. Confidence intervals are suppressed; successful-draw summaries are diagnostic only.")
    return {
        "status": status,
        "title": title,
        "message": message,
        "interpretation": (
            "Exploratory estimates only."
            if status not in ("converged", "complete")
            else "Interpret within the model, design and method's validated scope."
        ),
        "warnings": warnings,
    }


class AnalysisResult(dict):
    """A dict-compatible result with summaries, long rows and optional plots."""

    def summary(self):
        status = scientific_status(self)
        lines = [status["title"] + ": " + status["message"]] + status["warnings"]
        lines += [
            f"{name}: {q['value']:.6g} {q['unit']}"
            for name, q in self.get("parameters", {}).items()
        ]
        if "iterations" in self:
            lines.append(
                f"Iterations: {self['iterations']}; simulations: {self.get('simulations','unknown')}"
            )
        return "\n".join(lines)

    def rows(self):
        """Return independent long-format rows, preserving times, sides and units."""
        if "outputs" in self:
            return [
                {
                    "row": i,
                    "time": t,
                    "side": self["sides"][i],
                    "time_unit": self["time_unit"],
                    "output": c["name"],
                    "unit": c["unit"],
                    "value": c["values"][i],
                }
                for c in self["outputs"]
                for i, t in enumerate(self["times"])
            ]
        if "fitted_observations" in self:
            return deepcopy(self["fitted_observations"])
        fail("this result has no trajectory or fitted-observation table", "unsupported")

    def to_frame(self):
        """Create a pandas DataFrame; requires qspquest[notebook]."""
        try:
            import pandas as pd
        except ImportError as e:
            raise ImportError(
                "Install qspquest[notebook] for DataFrame helpers"
            ) from e
        return pd.DataFrame(self.rows())

    def plot(self, output=None, *, ax=None):
        """Plot a trajectory or observed/predicted fit; requires matplotlib."""
        try:
            import matplotlib.pyplot as plt
        except ImportError as e:
            raise ImportError("Install qspquest[notebook] for plotting") from e
        if ax is None:
            _, ax = plt.subplots()
        if "outputs" in self:
            names = [c["name"] for c in self["outputs"]]
            output = output or names[0]
            if output not in names:
                fail("unknown output")
            c = next(c for c in self["outputs"] if c["name"] == output)
            # Break lines at event-side duplicates rather than draw through jumps.
            times, values = [], []
            for i, (time, value) in enumerate(zip(self["times"], c["values"])):
                if i and time == self["times"][i - 1]:
                    times.append(float("nan"))
                    values.append(float("nan"))
                times.append(time)
                values.append(value)
            ax.plot(times, values, label=output)
            ax.set(
                xlabel=f"Time ({self['time_unit']})", ylabel=f"{output} ({c['unit']})"
            )
        else:
            rows = [
                r
                for r in self.rows()
                if not r.get("censoring") and (output is None or r["output"] == output)
            ]
            if not rows:
                fail("no quantifiable rows for this output")
            if len({(r["output"], r["unit"]) for r in rows}) != 1:
                fail("select one output to plot")
            ax.scatter(
                [r.get("individual_prediction", r["prediction"]) for r in rows],
                [r["observed"] for r in rows],
                label="observations",
            )
            ax.set(
                xlabel=f"Predicted ({rows[0]['unit']})",
                ylabel=f"Observed ({rows[0]['unit']})",
            )
        ax.legend()
        return ax


def analysis_subjects(dataset, *, covariates=None):
    """Convert a PopulationFitDataset to native-simulation analysis subjects.

    Keeps dosing and source-row mapping. Residual errors are supplied separately
    to fit_population; endpoint error contracts remain available on the dataset.
    """
    from .dataset import PopulationFitDataset

    if not isinstance(dataset, PopulationFitDataset):
        fail("expected PopulationFitDataset")
    subjects = dataset.request["problem"]["request"]["subjects"]
    source = {
        (r["subject_index"], r["observation_index"]): r["source_row"]
        for r in dataset.mapping
    }
    result = []
    for i, (identifier, s) in enumerate(zip(dataset.subject_ids, subjects)):
        if s.get("priors"):
            fail("analysis subjects do not support priors", "unsupported")
        obs = [
            {k: deepcopy(v) for k, v in o.items() if k != "error"}
            for o in s["observations"]
        ]
        for j, o in enumerate(obs):
            o["source_row"] = source[(i, j)]
        result.append(
            {
                "id": identifier,
                "run": deepcopy(s["simulation"]["run"]),
                "observations": obs,
                "covariates": deepcopy((covariates or {}).get(identifier, {})),
            }
        )
    if covariates is not None and set(covariates) - set(dataset.subject_ids):
        fail("unknown covariate subject ID")
    return result


def fit_population_dataset(
    model, dataset, parameters, omega, error, *, covariates_by_subject=None, **options
):
    """Fit prepared NONMEM records with an explicit analysis error contract."""
    return model.fit_population(
        analysis_subjects(dataset, covariates=covariates_by_subject),
        parameters,
        omega,
        error,
        **options,
    )
