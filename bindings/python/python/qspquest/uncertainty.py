"""Likelihood profiles and subject-resampling bootstrap with failure accounting."""

from copy import deepcopy
import math
from ._analysis import (
    Budget,
    bind,
    dependencies,
    error_spec,
    fail,
    finite,
    identity,
    initial_error,
    integer,
    likelihood,
    observations,
    parameter_specs,
)
from .estimation import fit_data
from .population import fit_population


def profile_likelihood(
    model,
    run,
    observations_data,
    parameters,
    error,
    parameter,
    grid,
    *,
    confidence=0.95,
    max_simulations=20000,
    max_iterations=300,
):
    """Reoptimize every nuisance parameter and free SD at each fixed grid point.

    Intervals are grid-interpolated likelihood-ratio approximations. Open ends
    are reported as null; they are never replaced by search bounds.
    """
    np, optimize, _, stats = dependencies()
    specs, err = parameter_specs(parameters), error_spec(error)
    if parameter not in specs:
        fail("profile parameter is not selected for fitting")
    if not 0 < finite(confidence, "confidence") < 1:
        fail("confidence must be in (0, 1)")
    values = np.asarray(grid, dtype=float)
    if (
        values.ndim != 1
        or not 3 <= len(values) <= 200
        or not np.all(np.isfinite(values))
        or np.any(np.diff(values) <= 0)
    ):
        fail("profile grid must contain 3..200 strictly increasing finite values")
    if values[0] < specs[parameter]["lower"] or values[-1] > specs[parameter]["upper"]:
        fail("profile grid exceeds fitted parameter bounds")
    budget = Budget(max_simulations)
    optimum = fit_data(
        model,
        run,
        observations_data,
        parameters,
        error,
        max_iterations=max_iterations,
        _budget=budget,
    )
    if optimum["status"] != "converged":
        fail("profile requires a converged unrestricted fit", "optimizer")
    remaining = {
        k: {**v, "initial": optimum["parameters"][k]["value"]}
        for k, v in specs.items()
        if k != parameter
    }
    starting_error = deepcopy(err)
    for k in ("additive", "proportional"):
        if isinstance(starting_error[k], dict):
            starting_error[k]["initial"] = optimum["error"][k]
    records = []
    for value in values:
        fixed_run = bind(run, specs, {parameter: float(value)})
        try:
            if remaining:
                fitted = fit_data(
                    model,
                    fixed_run,
                    observations_data,
                    remaining,
                    starting_error,
                    max_iterations=max_iterations,
                    _budget=budget,
                )
                objective, status = fitted["objective"], fitted["status"]
                nuisance = fitted["parameters"]
            else:
                result = budget.run(model, fixed_run)
                y, prediction, censored = observations(result, observations_data)
                keys = [
                    k for k in ("additive", "proportional") if isinstance(err[k], dict)
                ]

                def f(log_sd):
                    e = initial_error(starting_error)
                    for k, v in zip(keys, np.exp(log_sd)):
                        e[k] = v
                    return likelihood(y, prediction, censored, e)[0]

                if keys:
                    residual_fit = optimize.minimize(
                        f,
                        [math.log(optimum["error"][k]) for k in keys],
                        bounds=[
                            (math.log(err[k]["lower"]), math.log(err[k]["upper"]))
                            for k in keys
                        ],
                        method="L-BFGS-B",
                        options={"maxiter": max_iterations},
                    )
                    objective, status = float(residual_fit.fun), (
                        "converged" if residual_fit.success else "optimizer_failed"
                    )
                else:
                    objective, status = f([]), "converged"
                nuisance = {}
            records.append(
                {
                    "value": float(value),
                    "objective": objective,
                    "delta_nll": objective - optimum["objective"],
                    "status": status,
                    "nuisance_parameters": nuisance,
                }
            )
        except ValueError as exception:
            if getattr(exception, "code", None) == "budget":
                records.append(
                    {
                        "value": float(value),
                        "objective": None,
                        "delta_nll": None,
                        "status": "evaluation_limit",
                    }
                )
            elif getattr(exception, "code", None) in ("domain", "solver", "optimizer"):
                records.append(
                    {
                        "value": float(value),
                        "objective": None,
                        "delta_nll": None,
                        "status": "failed",
                        "message": str(exception),
                    }
                )
            else:
                raise
    threshold = float(stats.chi2.ppf(confidence, 1) / 2)
    estimate = optimum["parameters"][parameter]["value"]
    augmented = sorted(
        records + [{"value": estimate, "delta_nll": 0.0, "status": "converged"}],
        key=lambda r: r["value"],
    )
    crossings = []
    for a, b in zip(augmented, augmented[1:]):
        if a["status"] != "converged" or b["status"] != "converged":
            continue
        da, db = a["delta_nll"] - threshold, b["delta_nll"] - threshold
        if da * db <= 0 and da != db:
            crossings.append(a["value"] + (b["value"] - a["value"]) * (-da) / (db - da))
    left, right = [v for v in crossings if v <= estimate], [
        v for v in crossings if v >= estimate
    ]
    return {
        "schema": "qspquest.profile/v0.1",
        "parameter": parameter,
        "estimate": estimate,
        "unit": specs[parameter]["unit"],
        "confidence": confidence,
        "delta_nll_threshold": threshold,
        "interval": [max(left) if left else None, min(right) if right else None],
        "interval_method": "grid_interpolated_likelihood_ratio",
        "status": (
            "complete"
            if all(r["status"] == "converged" for r in records)
            else "incomplete"
        ),
        "profile": records,
        "unrestricted_fit": optimum,
        "simulations": budget.used,
        "identity": identity(
            model,
            {
                "run": run,
                "observations": observations_data,
                "parameters": parameters,
                "error": error,
                "parameter": parameter,
                "grid": list(grid),
                "confidence": confidence,
                "max_simulations": max_simulations,
                "max_iterations": max_iterations,
            },
        ),
    }


def bootstrap_population(
    model,
    subjects,
    parameters,
    omega,
    error,
    *,
    replicates=100,
    confidence=0.95,
    seed=1009,
    minimum_success_fraction=0.8,
    max_simulations=1000000,
    control=None,
    **fit_options,
):
    """Resample whole subjects, retaining dose histories and covariates together.

    Every replicate has a distinct recorded seed, source-subject map, status,
    boundary flags and failure message. Percentile intervals require the stated
    success fraction. No coverage calibration is implied by finite intervals.
    """
    np, _, _, _ = dependencies()
    integer(replicates, "replicates", 2, 1000)
    integer(seed, "seed", 0, 2**64 - 1)
    if (
        not 0 < finite(confidence, "confidence") < 1
        or not 0 < finite(minimum_success_fraction, "minimum_success_fraction") <= 1
    ):
        fail(
            "confidence and minimum_success_fraction must be in (0, 1] with confidence < 1"
        )
    if not isinstance(subjects, list) or not subjects:
        fail("subjects are required")
    if "_budget" in fit_options or "seed" in fit_options:
        fail("bootstrap controls fit seeds and the aggregate budget")
    budget, rng, records = (
        Budget(max_simulations, control),
        np.random.default_rng(seed),
        [],
    )
    if control is not None:
        from .control import restore_budget

        # Validate the native identity with the same initial bindings as fitting.
        # Subject runs may contain values that the selected fitted parameters override.
        specs = parameter_specs(parameters)
        probe = budget.run(
            model,
            bind(
                subjects[0]["run"],
                specs,
                {name: spec["initial"] for name, spec in specs.items()},
            ),
        )
        context = identity(
            model,
            {
                "subjects": subjects,
                "parameters": parameters,
                "omega": omega,
                "error": error,
                "replicates": replicates,
                "confidence": confidence,
                "seed": seed,
                "minimum_success_fraction": minimum_success_fraction,
                "max_simulations": max_simulations,
                "fit_options": fit_options,
            },
        )
        saved = control.start("bootstrap", context, probe["identity"])
        if saved is not None:
            restore_budget(budget, saved["simulations"])
            records = saved["records"]
            rng.bit_generator.state = saved["rng"]
    for replicate in range(len(records), replicates):
        indices = rng.integers(0, len(subjects), len(subjects))
        fit_seed = int(rng.integers(0, 2**63 - 1))
        cohort = []
        for new_id, index in enumerate(indices):
            subject = deepcopy(subjects[int(index)])
            subject["source_subject_id"] = subject.get("id", int(index))
            subject["id"] = f"bootstrap-{new_id}"
            cohort.append(subject)
        record = {
            "replicate": replicate,
            "seed": fit_seed,
            "source_indices": indices.tolist(),
            "parameters": None,
            "boundary_parameters": [],
        }
        try:
            result = fit_population(
                model,
                cohort,
                parameters,
                omega,
                error,
                seed=fit_seed,
                _budget=budget,
                **fit_options,
            )
            record.update(
                status=result["status"],
                parameters=result["parameters"],
                omega=result["omega"],
                error=result["error"],
                covariate_effects=result["covariate_effects"],
                boundary_parameters=result["boundary_parameters"],
                boundary_residual_errors=result["boundary_residual_errors"],
                boundary_covariate_coefficients=result[
                    "boundary_covariate_coefficients"
                ],
                boundary_variances=result["boundary_variances"],
            )
        except ValueError as exception:
            if getattr(exception, "code", None) == "cancelled":
                raise
            record.update(
                status="failed",
                code=getattr(exception, "code", "invalid_input"),
                message=str(exception),
            )
        records.append(record)
        if control is not None:
            control.boundary(
                replicate + 1,
                replicates,
                budget,
                {
                    "records": records,
                    "rng": rng.bit_generator.state,
                    "simulations": budget.used,
                },
                final=replicate + 1 == replicates,
            )
    successful = [r for r in records if r["status"] == "converged"]
    intervals = None
    if len(successful) >= max(2, math.ceil(replicates * minimum_success_fraction)):
        alpha = (1 - confidence) / 2
        intervals = {
            name: {
                "unit": spec["unit"],
                "bounds": np.quantile(
                    [r["parameters"][name]["value"] for r in successful],
                    [alpha, 1 - alpha],
                ).tolist(),
            }
            for name, spec in parameters.items()
        }
    nuisance_intervals = None
    if intervals is not None:
        nuisance_intervals = {
            "error": {
                key: np.quantile(
                    [r["error"][key] for r in successful], [alpha, 1 - alpha]
                ).tolist()
                for key in ("additive", "proportional")
            },
            "omega": np.quantile(
                [r["omega"] for r in successful], [alpha, 1 - alpha], axis=0
            ).tolist(),
            "covariates": [
                np.quantile(
                    [r["covariate_effects"][j]["estimate"] for r in successful],
                    [alpha, 1 - alpha],
                ).tolist()
                for j in range(len(successful[0]["covariate_effects"]))
            ],
        }
    return {
        "schema": "qspquest.bootstrap/v0.1",
        "method": "subject_percentile_bootstrap",
        "status": (
            "complete"
            if intervals is not None
            else "insufficient_successful_replicates"
        ),
        "confidence": confidence,
        "successful_replicates": len(successful),
        "failed_replicates": replicates - len(successful),
        "replicates": records,
        "intervals": intervals,
        "nuisance_intervals": nuisance_intervals,
        "simulations": budget.used,
        "identity": identity(
            model,
            {
                "subjects": subjects,
                "parameters": parameters,
                "omega": omega,
                "error": error,
                "replicates": replicates,
                "confidence": confidence,
                "seed": seed,
                "minimum_success_fraction": minimum_success_fraction,
                "max_simulations": max_simulations,
                "fit_options": fit_options,
            },
        ),
    }
