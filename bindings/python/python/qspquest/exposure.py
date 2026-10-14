"""Sampled-trajectory exposure summaries and paired population comparisons."""

from ._analysis import (
    Budget,
    bind,
    columns,
    dependencies,
    fail,
    finite,
    identity,
    integer,
    parameter_specs,
)
import math


def exposure_summary(result, output, *, threshold=None):
    """Integrate the sampled piecewise-linear trajectory without extrapolation.

    AUC and threshold duration are conditional on the supplied observation grid;
    this is not terminal-phase NCA or AUC-to-infinity. Duplicate times must be
    ordered pre/post so no trapezoid spans a dose jump.
    """
    np, _, _, _ = dependencies()
    data = columns(result)
    if output not in data:
        fail("unknown exposure output")
    y, unit = data[output]
    t = np.asarray(result["times"], dtype=float)
    sides = result["sides"]
    if (
        len(t) < 2
        or len(y) != len(t)
        or len(sides) != len(t)
        or not np.all(np.isfinite(t))
        or not np.all(np.isfinite(y))
        or np.any(np.diff(t) < 0)
        or t[-1] <= t[0]
    ):
        fail("exposure requires a finite ordered trajectory with nonzero duration")
    for i, duration in enumerate(np.diff(t)):
        if duration == 0 and (sides[i], sides[i + 1]) != ("pre", "post"):
            fail("duplicate exposure times must be ordered pre then post")
    dt = np.diff(t)
    area = float(np.dot(dt, (y[:-1] + y[1:]) / 2))
    peak = int(np.argmax(y))
    above = None
    if threshold is not None:
        cutoff = finite(threshold, "threshold")
        duration_above = 0.0
        for duration, a, b in zip(dt, y[:-1] - cutoff, y[1:] - cutoff):
            if a >= 0 and b >= 0:
                duration_above += duration
            elif a < 0 and b < 0:
                continue
            elif a != b:
                fraction = -a / (b - a)
                duration_above += duration * (1 - fraction if b > a else fraction)
        above = {
            "threshold": cutoff,
            "duration": float(duration_above),
            "fraction_of_window": float(duration_above / (t[-1] - t[0])),
        }
    return {
        "output": output,
        "unit": unit,
        "time_unit": result["time_unit"],
        "window": [float(t[0]), float(t[-1])],
        "auc": area,
        "auc_unit": f"{unit} * {result['time_unit']}",
        "cmax": float(y[peak]),
        "tmax": float(t[peak]),
        "peak_side": sides[peak],
        "cmin": float(y.min()),
        "endpoint": float(y[-1]),
        "endpoint_side": sides[-1],
        "time_above_threshold": above,
        "method": "sampled_piecewise_linear_no_extrapolation",
    }


def compare_regimens(
    model,
    regimens,
    parameters,
    omega,
    output,
    *,
    random_effects=None,
    subjects=100,
    seed=1009,
    threshold=None,
    percentiles=(0.05, 0.5, 0.95),
    parameter_draws=None,
    max_simulations=50000,
):
    """Use the same virtual subjects for every regimen and return paired metrics.

    ``parameter_draws`` can propagate bootstrap parameter uncertainty separately
    from IIV. Concentration curves exclude measurement error. Endpoint means a
    trough only when the request samples the appropriate pre-dose boundary.
    """
    np, _, _, _ = dependencies()
    integer(subjects, "subjects", 1, 2000)
    integer(seed, "seed", 0, 2**64 - 1)
    if not isinstance(regimens, dict) or not 1 <= len(regimens) <= 25:
        fail("regimens must contain 1..25 named native requests")
    specs = parameter_specs(
        {
            name: {
                "initial": q["value"],
                "lower": max(q["value"] * 0.001, 1e-15),
                "upper": q["value"] * 1000,
                "unit": q["unit"],
            }
            for name, q in parameters.items()
        }
    )
    names = list(parameters) if random_effects is None else list(random_effects)
    if (
        not 1 <= len(names) <= 8
        or len(set(names)) != len(names)
        or any(k not in specs for k in names)
    ):
        fail("random effects must select 1..8 distinct parameters")
    om = np.asarray(omega, dtype=float)
    if (
        om.shape != (len(names), len(names))
        or not np.all(np.isfinite(om))
        or not np.allclose(om, om.T, atol=1e-12, rtol=0)
    ):
        fail("Omega must be finite symmetric covariance")
    try:
        chol = np.linalg.cholesky(om)
    except np.linalg.LinAlgError:
        fail("Omega must be positive definite")
    p = np.asarray(percentiles, dtype=float)
    if (
        p.ndim != 1
        or not 1 <= len(p) <= 10
        or not np.all(np.isfinite(p))
        or np.any(p <= 0)
        or np.any(p >= 1)
        or np.any(np.diff(p) <= 0)
    ):
        fail("percentiles must be increasing values in (0, 1)")
    if parameter_draws is not None:
        if (
            not isinstance(parameter_draws, list)
            or not 1 <= len(parameter_draws) <= 1000
        ):
            fail("parameter_draws must contain 1..1000 quantity mappings")
        for draw in parameter_draws:
            if set(draw) != set(parameters) or any(
                draw[k].get("unit") != parameters[k]["unit"]
                or finite(draw[k].get("value"), k) <= 0
                for k in parameters
            ):
                fail("parameter draws must match selected parameter names and units")
    budget, rng = Budget(max_simulations), np.random.default_rng(seed)
    draws = []
    for _ in range(subjects):
        base = (
            parameters
            if parameter_draws is None
            else parameter_draws[int(rng.integers(len(parameter_draws)))]
        )
        values = {k: q["value"] for k, q in base.items()}
        for name, eta in zip(names, chol @ rng.normal(size=len(names))):
            values[name] *= math.exp(eta)
        draws.append(values)
    records, native_identity = {}, None
    for label, run in regimens.items():
        trajectories, metrics = [], []
        grid, unit = None, None
        for values in draws:
            result = budget.run(model, bind(run, specs, values))
            native_identity = result["identity"]
            current_grid = (result["times"], result["sides"])
            current_unit = columns(result).get(output, (None, None))[1]
            if grid is not None and (current_grid != grid or current_unit != unit):
                fail(
                    "population trajectories must share the observation grid and output unit"
                )
            grid, unit = current_grid, current_unit
            if subjects * len(grid[0]) > 2000000:
                fail("population trajectory exceeds bounded storage")
            metrics.append(exposure_summary(result, output, threshold=threshold))
            trajectories.append(columns(result)[output][0])
        summaries = {
            key: np.quantile([m[key] for m in metrics], p).tolist()
            for key in ("auc", "cmax", "tmax", "cmin", "endpoint")
        }
        if threshold is not None:
            summaries["time_above_threshold"] = np.quantile(
                [m["time_above_threshold"]["duration"] for m in metrics], p
            ).tolist()
        records[label] = {
            "times": grid[0],
            "sides": grid[1],
            "unit": unit,
            "time_unit": result["time_unit"],
            "percentiles": p.tolist(),
            "curve_percentiles": np.quantile(trajectories, p, axis=0).tolist(),
            "metric_percentiles": summaries,
            "subject_metrics": metrics,
        }
    reference = next(iter(records))
    for label, record in records.items():
        if (record["unit"], record["time_unit"]) != (
            records[reference]["unit"],
            records[reference]["time_unit"],
        ):
            fail("paired regimen metrics require matching output and time units")
        record["paired_differences_vs_reference"] = {
            key: np.quantile(
                [
                    m[key] - r[key]
                    for m, r in zip(
                        record["subject_metrics"], records[reference]["subject_metrics"]
                    )
                ],
                p,
            ).tolist()
            for key in ("auc", "cmax", "endpoint")
        }
    return {
        "schema": "qspquest.regimen-comparison/v0.1",
        "seed": seed,
        "subjects": subjects,
        "reference": reference,
        "regimens": records,
        "variability": (
            "iiv_and_parameter_draws" if parameter_draws is not None else "iiv_only"
        ),
        "simulations": budget.used,
        "identity": {
            **identity(
                model,
                {
                    "regimens": regimens,
                    "parameters": parameters,
                    "omega": omega,
                    "output": output,
                    "random_effects": names,
                    "subjects": subjects,
                    "seed": seed,
                    "threshold": threshold,
                    "percentiles": p.tolist(),
                    "parameter_draws": parameter_draws,
                    "max_simulations": max_simulations,
                },
            ),
            "native": native_identity,
        },
    }
