"""Likelihood profiles and subject-resampling bootstrap with failure accounting."""

from ._analysis import fail, in_declared_order, named, native


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
    result = native(
        model,
        "profile_likelihood",
        {
            "run": run,
            "observations": observations_data,
            "parameters": named(parameters, "parameters"),
            "error": error,
            "parameter": parameter,
            "grid": grid,
            "confidence": confidence,
            "max_simulations": max_simulations,
            "max_iterations": max_iterations,
        },
    )
    return in_declared_order(result, list(parameters))


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
    if "seed" in fit_options or "max_simulations" in fit_options:
        fail("bootstrap controls fit seeds and the aggregate budget")
    if "covariates" in fit_options:
        fit_options["covariates"] = list(fit_options["covariates"])
    result = native(
        model,
        "bootstrap_population",
        {
            "subjects": subjects,
            "parameters": named(parameters, "parameters"),
            "omega": omega,
            "error": error,
            "replicates": replicates,
            "confidence": confidence,
            "seed": seed,
            "minimum_success_fraction": minimum_success_fraction,
            "max_simulations": max_simulations,
            "fit_options": fit_options,
        },
        control,
    )
    return in_declared_order(result, list(parameters))
