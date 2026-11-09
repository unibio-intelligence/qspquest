"""General bounded SAEM orchestrated over QSPQuest native simulations."""

from ._analysis import in_declared_order, named, native


def fit_population(
    model,
    subjects,
    parameters,
    omega,
    error,
    *,
    covariates=(),
    covariance="full",
    variance_bounds=(1e-4, 2.0),
    seed=1009,
    max_iterations=600,
    burn_in=150,
    sweeps=3,
    tolerance=0.01,
    max_simulations=200000,
    control=None,
):
    """SAEM with 1..8 positive lognormal effects and exponential covariates.

    Each selected parameter has one random effect. Each subject supplies a
    native ``run``, ``observations`` and optional constant ``covariates``. Both
    SDs of normal combined error can be fitted jointly with covariates. M3
    censoring uses its probability in sampling and the analytic conditional
    second moment in the residual-error M step. Covariance is full or diagonal.

    The reported objective is a complete-data surrogate, never a marginal OFV.
    Convergence is a stationarity/mixing screen, not proof of a global optimum.
    """
    result = native(
        model,
        "fit_population",
        {
            "subjects": subjects,
            "parameters": named(parameters, "parameters"),
            "omega": omega,
            "error": error,
            "covariates": list(covariates),
            "covariance": covariance,
            "variance_bounds": variance_bounds,
            "seed": seed,
            "max_iterations": max_iterations,
            "burn_in": burn_in,
            "sweeps": sweeps,
            "tolerance": tolerance,
            "max_simulations": max_simulations,
        },
        control,
    )
    return in_declared_order(result, list(parameters))
