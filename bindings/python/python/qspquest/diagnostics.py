"""Serializable fit diagnostics and seeded population predictive checks."""

from ._analysis import in_declared_order, named, native


def fit_diagnostics(fit):
    """Return plotting rows, residual summaries, mixing and boundary warnings.

    Standardized residuals are conditional residuals, not CWRES or NPDE.
    Censored rows retain null residuals and their quantification limits.
    """
    result = native(None, "fit_diagnostics", {"fit": fit})
    names = fit.get("parameters") if isinstance(fit, dict) else None
    return in_declared_order(result, list(names) if isinstance(names, dict) else ())


def visual_predictive_check(
    model,
    subjects,
    parameters,
    omega,
    error,
    *,
    random_effects=None,
    covariates=(),
    replicates=100,
    bins=None,
    percentiles=(0.05, 0.5, 0.95),
    confidence=0.95,
    prediction_corrected=False,
    stratify=(),
    seed=1009,
    max_simulations=50000,
    control=None,
    workers=1,
):
    """Simulate population effects and observation error on the original design.

    Retains each subject's regimen, fixed inputs and sampling sides. BLQ rates
    are checked separately; concentration quantiles use only quantifiable values
    in both observed and simulated data. Limits must be supplied for every row
    for a fully censored-data-aware check. Uncertainty is Monte Carlo uncertainty
    of replicate quantiles, not fitted-parameter uncertainty.
    Each replicate draws from its own random substream, so results do not
    depend on ``workers`` (1..8, native threads). Failed replicates remain in
    the denominator and suppress intervals.
    """
    return native(
        model,
        "visual_predictive_check",
        {
            "subjects": subjects,
            "parameters": named(parameters, "parameters"),
            "omega": omega,
            "error": error,
            "random_effects": None if random_effects is None else list(random_effects),
            "covariates": list(covariates),
            "replicates": replicates,
            "bins": bins,
            "percentiles": percentiles,
            "confidence": confidence,
            "prediction_corrected": prediction_corrected,
            "stratify": list(stratify),
            "seed": seed,
            "max_simulations": max_simulations,
            "workers": workers,
        },
        control,
    )
