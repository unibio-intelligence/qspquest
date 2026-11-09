"""Sampled-trajectory exposure summaries and paired population comparisons."""

from ._analysis import fail, in_declared_order, named, native


def exposure_summary(result, output, *, threshold=None):
    """Integrate the sampled piecewise-linear trajectory without extrapolation.

    AUC and threshold duration are conditional on the supplied observation grid;
    this is not terminal-phase NCA or AUC-to-infinity. Duplicate times must be
    ordered pre/post so no trapezoid spans a dose jump.
    """
    return native(
        None,
        "exposure_summary",
        {"result": dict(result), "output": output, "threshold": threshold},
    )


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
    workers=1,
):
    """Use the same virtual subjects for every regimen and return paired metrics.

    ``parameter_draws`` can propagate bootstrap parameter uncertainty separately
    from IIV. Concentration curves exclude measurement error. Endpoint means a
    trough only when the request samples the appropriate pre-dose boundary.
    The first regimen is the reference for paired differences.
    """
    if not isinstance(regimens, dict):
        fail("regimens must be a mapping of labels to native requests")
    result = native(
        model,
        "compare_regimens",
        {
            "regimens": [{"label": k, "run": v} for k, v in regimens.items()],
            "parameters": named(parameters, "parameters"),
            "omega": omega,
            "output": output,
            "random_effects": None if random_effects is None else list(random_effects),
            "subjects": subjects,
            "seed": seed,
            "threshold": threshold,
            "percentiles": percentiles,
            "parameter_draws": None if parameter_draws is None else list(parameter_draws),
            "max_simulations": max_simulations,
            "workers": workers,
        },
    )
    return in_declared_order(result, list(parameters), list(regimens))
