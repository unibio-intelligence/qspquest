"""Bounded deterministic maximum likelihood using the native simulator."""

from copy import deepcopy
import math
from ._analysis import (
    Budget,
    bind,
    dependencies,
    error_spec,
    fail,
    identity,
    initial_error,
    integer,
    likelihood,
    observations,
    parameter_specs,
    scalar_parameters,
)


def fit_data(
    model,
    run,
    observations_data,
    parameters,
    error,
    *,
    max_simulations=2000,
    max_iterations=300,
    tolerance=1e-7,
    method="L-BFGS-B",
    _budget=None
):
    """Fit bounded parameters and additive/proportional SDs by native simulation.

    Event parameters select quantity-valued paths in the run request. The normal
    combined-error model adds variances. Lognormal SD is on log concentration.
    All observations sharing this error must have the same unit and output.
    """
    np, optimize, _, _ = dependencies()
    specs, err = parameter_specs(parameters), error_spec(error)
    integer(max_iterations, "max_iterations", 1, 10000)
    if not math.isfinite(tolerance) or tolerance <= 0:
        fail("tolerance must be finite and positive")
    if method not in ("L-BFGS-B", "Powell"):
        fail("method must be L-BFGS-B or Powell")
    budget = _budget or Budget(max_simulations)
    before = budget.used
    names = list(specs)
    axes = [(item["lower"], item["upper"], item["initial"]) for item in specs.values()]
    error_axes = [
        key for key in ("additive", "proportional") if isinstance(err[key], dict)
    ]
    axes += [
        (
            math.log(err[k]["lower"]),
            math.log(err[k]["upper"]),
            math.log(err[k]["initial"]),
        )
        for k in error_axes
    ]
    lower, width, start = map(np.asarray, zip(*[(a, b - a, c) for a, b, c in axes]))
    trace, best = [], [float("inf"), None]

    def unpack(z):
        values = lower + width * z
        fixed = dict(zip(names, values[: len(names)]))
        e = initial_error(err)
        for i, key in enumerate(error_axes):
            e[key] = math.exp(values[len(names) + i])
        return fixed, e

    def evaluate(z, strict=False):
        fixed, e = unpack(z)
        try:
            result = budget.run(model, bind(run, specs, fixed))
            y, prediction, censored = observations(result, observations_data)
            if len({row["output"] for row in observations_data}) != 1:
                fail("one shared residual-error contract requires one observed output")
            if np.all(censored) and error_axes:
                fail("all-censored data cannot estimate residual error")
            objective = likelihood(y, prediction, censored, e)[0]
        except ValueError as exception:
            code = getattr(exception, "code", None)
            if strict or code not in ("domain", "invalid_input", "integrity", "solver"):
                raise
            return 1e100
        entry = {
            "simulation": budget.used - before,
            "objective": objective,
            "parameters": {n: float(v) for n, v in fixed.items()},
            "error": e.copy(),
        }
        trace.append(entry)
        if objective < best[0]:
            best[:] = [objective, (fixed, e, result, y, prediction, censored)]
        return objective

    initial = evaluate((start - lower) / width, strict=True)
    status, message, iterations = (
        "evaluation_limit",
        "aggregate simulation budget exhausted",
        0,
    )
    try:
        fitted = optimize.minimize(
            evaluate,
            (start - lower) / width,
            method=method,
            bounds=[(0.0, 1.0)] * len(axes),
            options={
                "maxiter": max_iterations,
                "ftol": tolerance,
                **({"gtol": tolerance} if method == "L-BFGS-B" else {}),
            },
        )
        status = "converged" if fitted.success else "iteration_limit"
        message, iterations = str(fitted.message), int(fitted.nit)
    except ValueError as exception:
        if getattr(exception, "code", None) != "budget":
            raise
    fixed, e, result, y, prediction, censored = best[1]
    _, residual, sd = likelihood(y, prediction, censored, e)
    rows = []
    for i, observation in enumerate(observations_data):
        row = observation["row"]
        rows.append(
            {
                "row": row,
                "output": observation["output"],
                **(
                    {"source_row": observation["source_row"]}
                    if "source_row" in observation
                    else {}
                ),
                "time": result["times"][row],
                "side": result["sides"][row],
                "unit": observation["value"]["unit"],
                "observed": float(y[i]),
                "prediction": float(prediction[i]),
                "censoring": observation.get("censoring"),
                "residual": None if censored[i] else float(residual[i]),
                "standardized_residual": (
                    None if censored[i] else float(residual[i] / sd[i])
                ),
            }
        )
    return {
        "schema": "qspquest.analysis-fit/v0.1",
        "method": method,
        "status": status,
        "message": message,
        "parameters": scalar_parameters(specs, fixed),
        "error": e,
        "objective": best[0],
        "initial_objective": initial,
        "objective_kind": "negative_log_likelihood",
        "simulations": budget.used - before,
        "iterations": iterations,
        "trace": trace,
        "fitted_observations": rows,
        "boundary_parameters": [
            name
            for name, item in specs.items()
            if min(fixed[name] - item["lower"], item["upper"] - fixed[name])
            <= 1e-6 * (item["upper"] - item["lower"])
        ],
        "boundary_residual_errors": [
            key
            for key in error_axes
            if min(e[key] - err[key]["lower"], err[key]["upper"] - e[key])
            <= 1e-6 * (err[key]["upper"] - err[key]["lower"])
        ],
        "identity": {
            **identity(
                model,
                {
                    "run": run,
                    "observations": observations_data,
                    "parameters": parameters,
                    "error": error,
                    "method": method,
                    "tolerance": tolerance,
                    "max_iterations": max_iterations,
                    "max_simulations": max_simulations,
                },
            ),
            "native": result["identity"],
        },
    }
