"""Checked numerical derivatives of complete native regimens."""

from ._analysis import in_declared_order, named, native


def event_sensitivities(
    model,
    run,
    parameters,
    *,
    relative_step=1e-3,
    relative_tolerance=0.02,
    absolute_tolerance=1e-6,
    max_simulations=1000,
):
    """Differentiate parameter-dependent lag and explicit event-time controls.

    Runs the full event driver at two central-difference step sizes and reports
    the half-step derivative only when they agree. Coincident observation/event
    discontinuities, changing result grids, and unstable derivatives fail with
    no partial derivative. This is numerical differentiation, not saltation or
    symbolic moving-boundary differentiation.
    """
    result = native(
        model,
        "event_sensitivities",
        {
            "run": run,
            "parameters": named(parameters, "parameters"),
            "relative_step": relative_step,
            "relative_tolerance": relative_tolerance,
            "absolute_tolerance": absolute_tolerance,
            "max_simulations": max_simulations,
        },
    )
    return in_declared_order(result, list(parameters))
