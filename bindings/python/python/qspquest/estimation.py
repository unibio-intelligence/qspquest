"""Bounded deterministic maximum likelihood using the native simulator."""

from ._analysis import in_declared_order, named, native


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
    method="gradient",
):
    """Fit bounded parameters and additive/proportional SDs by native simulation.

    Event parameters select quantity-valued paths in the run request. The normal
    combined-error model adds variances. Lognormal SD is on log concentration.
    All observations sharing this error must have the same unit and output.
    ``method="gradient"`` uses projected BFGS with central-difference
    gradients; ``method="derivative_free"`` uses a compass pattern search.
    """
    result = native(
        model,
        "fit_data",
        {
            "run": run,
            "observations": observations_data,
            "parameters": named(parameters, "parameters"),
            "error": error,
            "max_simulations": max_simulations,
            "max_iterations": max_iterations,
            "tolerance": tolerance,
            "method": method,
        },
    )
    return in_declared_order(result, list(parameters))
