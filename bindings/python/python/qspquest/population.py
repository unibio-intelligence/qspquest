"""General bounded SAEM orchestrated over QSPQuest native simulations."""

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
    scalar_parameters,
)


def _stability_history(trace, burn_in):
    """Retain at least one unit of cumulative post-burn SA gain."""
    selected, span, later_gain = [], 0.0, None
    for record in reversed(trace):
        post_burn_iteration = record["iteration"] - burn_in
        if post_burn_iteration <= 0:
            break
        if later_gain is not None:
            span += later_gain
        selected.append(record["coordinates"])
        later_gain = post_burn_iteration ** -0.7
        if len(selected) >= 50 and span >= 1.0:
            return list(reversed(selected))
    return None


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
    _budget=None
):
    """SAEM with 1..8 positive lognormal effects and exponential covariates.

    Each selected parameter has one random effect. Each subject supplies a
    native ``run``, ``observations`` and optional constant ``covariates``. Both
    SDs of normal combined error can be fitted jointly with covariates. M3
    censoring uses its probability in sampling and conditional latent draws
    in the residual-error M step. Covariance is full or diagonal.

    The reported objective is a complete-data surrogate, never a marginal OFV.
    Convergence is a stationarity/mixing screen, not proof of a global optimum.
    """
    np, optimize, _, stats = dependencies()
    specs, err = parameter_specs(parameters), error_spec(error)
    names, dimension = list(specs), len(specs)
    if dimension > 8 or any(s["lower"] <= 0 or "path" in s for s in specs.values()):
        fail(
            "SAEM supports 1..8 positive model parameters; event controls are deterministic-fit only"
        )
    if not isinstance(subjects, list) or not dimension + 2 <= len(subjects) <= 128:
        fail("SAEM needs at least dimension+2 subjects, maximum 128")
    if any(
        not isinstance(s, dict) or "run" not in s or "observations" not in s
        for s in subjects
    ):
        fail("each subject must provide run and observations")
    ids = [s.get("id", i) for i, s in enumerate(subjects)]
    if any(not isinstance(x, (str, int)) or isinstance(x, bool) for x in ids) or len(
        set(ids)
    ) != len(ids):
        fail("subject IDs must be distinct strings or integers")
    integer(seed, "seed", 0, 2**64 - 1)
    integer(max_iterations, "max_iterations", 40, 2000)
    integer(burn_in, "burn_in", 10, max_iterations - 20)
    integer(sweeps, "sweeps", 1, 20)
    if not 0 < finite(tolerance, "tolerance") <= 0.1:
        fail("SAEM tolerance must be in (0, .1]")
    if covariance not in ("full", "diagonal"):
        fail("covariance must be full or diagonal")
    bounds = np.asarray(variance_bounds, dtype=float)
    if bounds.shape == (2,):
        bounds = np.tile(bounds, (dimension, 1))
    if (
        bounds.shape != (dimension, 2)
        or not np.all(np.isfinite(bounds))
        or np.any(bounds[:, 0] <= 0)
        or np.any(bounds[:, 1] < bounds[:, 0])
    ):
        fail("variance_bounds must be positive ordered pairs")
    om = np.asarray(omega, dtype=float)
    if (
        om.shape != (dimension, dimension)
        or not np.all(np.isfinite(om))
        or not np.allclose(om, om.T, atol=1e-12, rtol=0)
    ):
        fail("Omega must be a finite symmetric dimension-by-dimension matrix")
    try:
        np.linalg.cholesky(om)
    except np.linalg.LinAlgError:
        fail("Omega must be positive definite")
    if np.any(np.diag(om) < bounds[:, 0]) or np.any(np.diag(om) > bounds[:, 1]):
        fail("initial Omega is outside variance bounds")
    if covariance == "diagonal" and np.any(om - np.diag(np.diag(om))):
        fail("diagonal covariance requires diagonal initial Omega")
    n = len(subjects)
    controls = deepcopy(list(covariates))
    if len(controls) > 8 or n <= len(controls) + dimension:
        fail(
            "at most eight covariate coefficients with an informative cohort are supported"
        )
    x = np.zeros((n, len(controls)))
    axes = []
    for col, covariate_control in enumerate(controls):
        if (
            set(covariate_control)
            != {"parameter", "covariate", "reference", "initial", "lower", "upper"}
            or covariate_control["parameter"] not in names
        ):
            fail(
                "covariate requires parameter, covariate, reference, initial, lower and upper"
            )
        axis = names.index(covariate_control["parameter"])
        lo, hi = finite(covariate_control["lower"], "covariate.lower"), finite(
            covariate_control["upper"], "covariate.upper"
        )
        if (
            not lo < hi
            or not lo <= finite(covariate_control["initial"], "covariate.initial") <= hi
        ):
            fail("invalid covariate coefficient bounds")
        reference = finite(covariate_control["reference"], "covariate.reference")
        for i, subject in enumerate(subjects):
            x[i, col] = (
                finite(
                    subject.get("covariates", {}).get(covariate_control["covariate"]),
                    "subject covariate",
                )
                - reference
            )
        if np.max(abs(x[:, col])) * max(abs(lo), abs(hi)) > 8:
            fail("covariate effect exceeds the supported log range")
        axes.append(axis)
    if len({(c["parameter"], c["covariate"]) for c in controls}) != len(controls):
        fail("duplicate covariate effects")
    for axis in set(axes):
        design = np.column_stack(
            [np.ones(n), x[:, [j for j, a in enumerate(axes) if a == axis]]]
        )
        if np.linalg.matrix_rank(design) != design.shape[1]:
            fail("covariate design is rank deficient")
    error_axes = [k for k in ("additive", "proportional") if isinstance(err[k], dict)]
    row_count = sum(len(s["observations"]) for s in subjects)
    if not 1 <= row_count <= 10000 or row_count * (max_iterations - burn_in) > 2000000:
        fail("cohort or stochastic expectation storage exceeds bounded limits")
    if len({o["output"] for s in subjects for o in s["observations"]}) != 1:
        fail("one residual error contract requires one observed output")
    if error_axes and all(
        o.get("censoring") == "left" for s in subjects for o in s["observations"]
    ):
        fail("all-censored cohorts cannot estimate residual error")
    rng = np.random.default_rng(seed)
    budget = _budget or Budget(max_simulations, control)
    before = budget.used
    log_bounds = [
        (math.log(specs[k]["lower"]), math.log(specs[k]["upper"])) for k in names
    ]
    coefficient_bounds = [(c["lower"], c["upper"]) for c in controls]
    coefficients = np.array(
        [math.log(specs[k]["initial"]) for k in names]
        + [c["initial"] for c in controls]
    )

    def location(coef):
        loc = np.tile(coef[:dimension], (n, 1))
        for j, axis in enumerate(axes):
            loc[:, axis] += x[:, j] * coef[dimension + j]
        return loc

    current_error = initial_error(err)
    loc = location(coefficients)
    phi = loc.copy()
    mean, second = phi.copy(), np.einsum("ni,nj->nij", phi, phi) + om
    prediction_cache, y_values, censored_values, likelihoods = [], [], [], []
    native_identity = None

    def evaluate(i, log_values):
        nonlocal native_identity
        values = dict(zip(names, np.exp(log_values)))
        result = budget.run(model, bind(subjects[i]["run"], specs, values))
        native_identity = result["identity"]
        y, prediction, censored = observations(result, subjects[i]["observations"])
        return (
            likelihood(y, prediction, censored, current_error)[0],
            prediction,
            y,
            censored,
            result,
        )

    for i in range(n):
        nll, pred, y, cens, _ = evaluate(i, phi[i])
        likelihoods.append(nll)
        prediction_cache.append(pred)
        y_values.append(y)
        censored_values.append(cens)
    y_flat, censored_flat = np.concatenate(y_values), np.concatenate(censored_values)
    proposed, accepted = np.zeros(dimension, int), np.zeros(dimension, int)
    subject_accepted = np.zeros((n, dimension), int)
    proposal_scales = np.ones(dimension) * 0.8
    expectation_predictions, expectation_y, expectation_weights = [], [], np.empty(0)
    trace, phi_history, stable_checks = [], [], 0
    status = "iteration_limit"
    start_iteration = 0
    if control is not None:
        from .control import restore_budget

        context = identity(
            model,
            {
                "subjects": subjects,
                "parameters": parameters,
                "omega": omega,
                "error": error,
                "covariates": controls,
                "covariance": covariance,
                "variance_bounds": bounds.tolist(),
                "seed": seed,
                "max_iterations": max_iterations,
                "burn_in": burn_in,
                "sweeps": sweeps,
                "tolerance": tolerance,
                "max_simulations": max_simulations,
                "convergence_window": "cumulative-gain/v1",
                "initial_chain_equilibration": "first-half-of-burn-in",
            },
        )
        saved = control.start("population_fit", context, native_identity)
        if saved is not None:
            restore_budget(budget, saved["simulations"])
            coefficients, phi, mean, second, om = [
                np.asarray(saved[k])
                for k in ("coefficients", "phi", "mean", "second", "omega")
            ]
            current_error = saved["error"]
            prediction_cache = [np.asarray(v) for v in saved["prediction_cache"]]
            likelihoods = saved["likelihoods"]
            proposed, accepted, subject_accepted, proposal_scales = [
                np.asarray(saved[k])
                for k in ("proposed", "accepted", "subject_accepted", "proposal_scales")
            ]
            expectation_predictions = [
                np.asarray(v) for v in saved["expectation_predictions"]
            ]
            expectation_y = [np.asarray(v) for v in saved["expectation_y"]]
            expectation_weights = np.asarray(saved["expectation_weights"])
            trace = saved["trace"]
            phi_history = [np.asarray(v) for v in saved["phi_history"]]
            stable_checks = saved["stable_checks"]
            rng.bit_generator.state = saved["rng"]
            start_iteration = saved["completed"]
            status = saved["status"]

    def checkpoint(completed, final=False):
        if control is not None:
            control.boundary(
                completed,
                max_iterations,
                budget,
                {
                    "completed": completed,
                    "simulations": budget.used,
                    "status": status,
                    "coefficients": coefficients.tolist(),
                    "phi": phi.tolist(),
                    "mean": mean.tolist(),
                    "second": second.tolist(),
                    "omega": om.tolist(),
                    "error": current_error,
                    "prediction_cache": [v.tolist() for v in prediction_cache],
                    "likelihoods": list(likelihoods),
                    "proposed": proposed.tolist(),
                    "accepted": accepted.tolist(),
                    "subject_accepted": subject_accepted.tolist(),
                    "proposal_scales": proposal_scales.tolist(),
                    "expectation_predictions": [
                        v.tolist() for v in expectation_predictions
                    ],
                    "expectation_y": [v.tolist() for v in expectation_y],
                    "expectation_weights": expectation_weights.tolist(),
                    "trace": trace,
                    "phi_history": [v.tolist() for v in phi_history],
                    "stable_checks": stable_checks,
                    "rng": rng.bit_generator.state,
                },
                final=final,
            )

    def record_iteration(iteration):
        snapshot = np.concatenate(
            [
                coefficients,
                np.log(np.diag(om)),
                om[np.tril_indices(dimension, -1)],
                [math.log(current_error[k]) for k in error_axes],
            ]
        )
        trace.append(
            {
                "iteration": iteration + 1,
                "coordinates": snapshot.tolist(),
                "fixed_effects": np.exp(coefficients[:dimension]).tolist(),
                "omega": om.tolist(),
                "error": current_error.copy(),
            }
        )

    for iteration in range(
        start_iteration, max_iterations if status != "converged" else start_iteration
    ):
        loc = location(coefficients)
        precision = np.linalg.inv(om)
        batch_proposed, batch_accepted = np.zeros(dimension, int), np.zeros(
            dimension, int
        )
        try:
            for i in range(n):
                for _ in range(sweeps):
                    for axis in range(dimension):
                        candidate = phi[i].copy()
                        candidate[axis] += (
                            proposal_scales[axis]
                            * math.sqrt(om[axis, axis])
                            * rng.normal()
                        )
                        batch_proposed[axis] += 1
                        # Native parameter/domain failures reject this proposal;
                        # they never turn into an apparent successful fit.
                        try:
                            candidate_nll, candidate_prediction, _, _, _ = evaluate(
                                i, candidate
                            )
                        except ValueError as exception:
                            if getattr(exception, "code", None) in (
                                "domain",
                                "invalid_input",
                                "integrity",
                                "solver",
                            ):
                                continue
                            raise
                        old_eta, new_eta = phi[i] - loc[i], candidate - loc[i]
                        log_ratio = (
                            likelihoods[i]
                            - candidate_nll
                            + 0.5
                            * (
                                old_eta @ precision @ old_eta
                                - new_eta @ precision @ new_eta
                            )
                        )
                        if math.log(max(rng.random(), 1e-300)) < log_ratio:
                            phi[i], likelihoods[i], prediction_cache[i] = (
                                candidate,
                                candidate_nll,
                                candidate_prediction,
                            )
                            batch_accepted[axis] += 1
                            if iteration >= burn_in:
                                subject_accepted[i, axis] += 1
        except ValueError as exception:
            if getattr(exception, "code", None) != "budget":
                raise
            # Return no unfinalized prediction rows when the aggregate budget ends.
            status = "evaluation_limit"
            break
        if iteration < burn_in:
            rate = batch_accepted / np.maximum(batch_proposed, 1)
            proposal_scales *= np.exp(np.clip(rate - 0.35, -0.2, 0.2))
            proposal_scales = np.clip(proposal_scales, 0.05, 3.0)
        else:
            proposed += batch_proposed
            accepted += batch_accepted
        # Initial conditional chains equilibrate before their draws update
        # population covariance, covariates or residual-error statistics.
        if iteration < burn_in // 2:
            record_iteration(iteration)
            checkpoint(iteration + 1, final=iteration + 1 == max_iterations)
            continue
        gain = 1.0 if iteration < burn_in else (iteration - burn_in + 1.0) ** -0.7
        mean += gain * (phi - mean)
        second += gain * (np.einsum("ni,nj->nij", phi, phi) - second)

        # The conditional normal M step is a bounded quadratic regression.
        # Whiten with the full precision, preserving cross-effect covariance.
        design = np.zeros((n, dimension, dimension + len(controls)))
        design[:, :, :dimension] = np.eye(dimension)
        for j, axis in enumerate(axes):
            design[:, axis, dimension + j] = x[:, j]
        whitening = np.linalg.cholesky(precision)
        matrix = np.einsum("nab,ac->ncb", design, whitening).reshape(n * dimension, -1)
        target = (mean @ whitening).reshape(-1)
        regression_bounds = np.asarray(log_bounds + coefficient_bounds)
        regression_fit = optimize.lsq_linear(
            matrix,
            target,
            bounds=(regression_bounds[:, 0], regression_bounds[:, 1]),
            method="bvls",
            tol=1e-10,
            max_iter=200,
        )
        if not regression_fit.success:
            fail("bounded covariate M step failed", "optimizer")
        coefficients = regression_fit.x
        loc = location(coefficients)
        om_candidate = (
            second
            - np.einsum("ni,nj->nij", mean, loc)
            - np.einsum("ni,nj->nij", loc, mean)
            + np.einsum("ni,nj->nij", loc, loc)
        ).mean(axis=0)
        om_candidate = (om_candidate + om_candidate.T) / 2
        if covariance == "diagonal":
            om_candidate = np.diag(np.diag(om_candidate))
        diag = np.diag(om_candidate)
        if np.any(diag <= 0):
            fail(
                "SAEM covariance became singular; use an informative design",
                "identifiability",
            )
        scales = np.sqrt(np.clip(diag, bounds[:, 0], bounds[:, 1]) / diag)
        om = om_candidate * np.outer(scales, scales)
        try:
            np.linalg.cholesky(om)
        except np.linalg.LinAlgError:
            fail(
                "SAEM covariance is singular; no ridge was substituted",
                "identifiability",
            )
        if error_axes:
            pred_flat = np.concatenate(prediction_cache)
            latent_y = y_flat.copy()
            if np.any(censored_flat):
                _, residual, sd = likelihood(
                    y_flat, pred_flat, censored_flat, current_error
                )
                upper = residual[censored_flat] / sd[censored_flat]
                z = stats.truncnorm.rvs(-np.inf, upper, random_state=rng)
                if current_error["model"] == "lognormal":
                    latent_y[censored_flat] = np.exp(
                        np.log(pred_flat[censored_flat]) + sd[censored_flat] * z
                    )
                else:
                    latent_y[censored_flat] = (
                        pred_flat[censored_flat] + sd[censored_flat] * z
                    )
            if gain == 1:
                expectation_predictions, expectation_y, expectation_weights = (
                    [],
                    [],
                    np.empty(0),
                )
            expectation_weights *= 1 - gain
            expectation_weights = np.append(expectation_weights, gain)
            expectation_predictions.append(pred_flat.copy())
            expectation_y.append(latent_y)
            pred_history, y_history = np.asarray(expectation_predictions), np.asarray(
                expectation_y
            )

            def residual_objective(log_sd):
                e = current_error.copy()
                for key, value in zip(error_axes, np.exp(log_sd)):
                    e[key] = value
                if e["model"] == "lognormal":
                    r = np.log(y_history) - np.log(pred_history)
                    variance = e["proportional"] ** 2
                else:
                    r = y_history - pred_history
                    variance = (
                        e["additive"] ** 2 + (e["proportional"] * pred_history) ** 2
                    )
                return float(
                    np.dot(
                        expectation_weights,
                        (0.5 * np.log(variance) + 0.5 * r * r / variance).sum(axis=1),
                    )
                )

            residual_fit = optimize.minimize(
                residual_objective,
                [math.log(current_error[k]) for k in error_axes],
                method="L-BFGS-B",
                bounds=[
                    (math.log(err[k]["lower"]), math.log(err[k]["upper"]))
                    for k in error_axes
                ],
            )
            if not residual_fit.success:
                fail("residual-error M step failed", "optimizer")
            for key, value in zip(error_axes, np.exp(residual_fit.x)):
                current_error[key] = float(value)
            for i in range(n):
                likelihoods[i] = likelihood(
                    y_values[i], prediction_cache[i], censored_values[i], current_error
                )[0]
        record_iteration(iteration)
        if iteration >= burn_in:
            phi_history.append(phi.copy())
        if len(phi_history) >= 50 and len(phi_history) % 10 == 0:
            history_values = _stability_history(trace, burn_in)
            if history_values is None:
                stable_checks = 0
                checkpoint(iteration + 1, final=iteration + 1 == max_iterations)
                continue
            history = np.asarray(history_values)
            end_count = len(history) * 2 // 5
            drift = np.max(
                abs(history[:end_count].mean(axis=0) - history[-end_count:].mean(axis=0))
            )
            mixing = accepted / np.maximum(proposed, 1)
            stable_checks = (
                stable_checks + 1 if drift <= tolerance and np.all(mixing > 0.05) else 0
            )
            if stable_checks >= 4 and np.all(subject_accepted > 0):
                status = "converged"
                checkpoint(iteration + 1, final=True)
                break
        checkpoint(iteration + 1, final=iteration + 1 == max_iterations)
    theta = dict(zip(names, np.exp(coefficients[:dimension])))
    eta = mean - location(coefficients)
    rows, individual_parameters = [], []
    # Predictions use posterior mean log parameters, explicitly not conditional modes.
    if status != "evaluation_limit":
        try:
            for i, subject in enumerate(subjects):
                _, ipred, y, cens, result = evaluate(i, mean[i])
                _, pred, _, _, _ = evaluate(i, location(coefficients)[i])
                individual_parameters.append(dict(zip(names, np.exp(mean[i]).tolist())))
                _, residual, sd = likelihood(y, ipred, cens, current_error)
                for j, observation in enumerate(subject["observations"]):
                    row = observation["row"]
                    rows.append(
                        {
                            "subject_id": ids[i],
                            "subject_index": i,
                            "observation_index": j,
                            **(
                                {"source_row": observation["source_row"]}
                                if "source_row" in observation
                                else {}
                            ),
                            "row": row,
                            "output": observation["output"],
                            "unit": observation["value"]["unit"],
                            "time": result["times"][row],
                            "side": result["sides"][row],
                            "observed": float(y[j]),
                            "prediction": float(pred[j]),
                            "individual_prediction": float(ipred[j]),
                            "censoring": observation.get("censoring"),
                            "residual": None if cens[j] else float(residual[j]),
                            "standardized_residual": (
                                None if cens[j] else float(residual[j] / sd[j])
                            ),
                        }
                    )
        except ValueError as exception:
            if getattr(exception, "code", None) != "budget":
                raise
            status, rows, individual_parameters = "evaluation_limit", [], []
    max_lag = []
    history = np.asarray(phi_history)
    for axis in range(dimension):
        correlations = []
        if len(history) > 4:
            for i in range(n):
                a, b = history[:-1, i, axis], history[1:, i, axis]
                if np.std(a) > 0 and np.std(b) > 0:
                    correlations.append(float(np.corrcoef(a, b)[0, 1]))
        max_lag.append(max(correlations) if correlations else None)
    if status == "converged" and any(v is None or v > 0.95 for v in max_lag):
        status = "mixing_failed"
    shrinkage = 1 - np.var(eta, axis=0, ddof=1) / np.diag(om)
    precision = np.linalg.inv(om)
    surrogate = float(
        sum(likelihoods)
        + 0.5
        * np.einsum(
            "ni,ij,nj->",
            phi - location(coefficients),
            precision,
            phi - location(coefficients),
        )
        + n / 2 * (np.linalg.slogdet(om)[1] + dimension * math.log(2 * math.pi))
    )
    return {
        "schema": "qspquest.population-analysis/v0.1",
        "method": "saem_native_simulation",
        "status": status,
        "parameters": scalar_parameters(specs, theta),
        "random_effects": names,
        "omega": om.tolist(),
        "error": current_error,
        "covariate_effects": [
            {**c, "estimate": float(coefficients[dimension + j])}
            for j, c in enumerate(controls)
        ],
        "eta": eta.tolist(),
        "subject_ids": ids,
        "individual_parameters": individual_parameters,
        "individual_estimate_kind": "posterior_mean_log_parameter",
        "objective": surrogate,
        "objective_kind": "saem_complete_data_surrogate",
        "simulations": budget.used - before,
        "iterations": len(trace),
        "trace": trace,
        "fitted_observations": rows,
        "diagnostics": {
            "acceptance_by_axis": (accepted / np.maximum(proposed, 1)).tolist(),
            "minimum_subject_accepted_by_axis": subject_accepted.min(axis=0).tolist(),
            "max_subject_lag_one_correlation": max_lag,
            "eta_variance_shrinkage": shrinkage.tolist(),
            "stable_checks": stable_checks,
        },
        "boundary_parameters": [
            name
            for name in names
            if min(
                theta[name] - specs[name]["lower"], specs[name]["upper"] - theta[name]
            )
            <= 1e-6 * (specs[name]["upper"] - specs[name]["lower"])
        ],
        "boundary_residual_errors": [
            key
            for key in error_axes
            if min(
                current_error[key] - err[key]["lower"],
                err[key]["upper"] - current_error[key],
            )
            <= 1e-6 * (err[key]["upper"] - err[key]["lower"])
        ],
        "boundary_covariate_coefficients": [
            j
            for j, c in enumerate(controls)
            if min(
                coefficients[dimension + j] - c["lower"],
                c["upper"] - coefficients[dimension + j],
            )
            <= 1e-6 * (c["upper"] - c["lower"])
        ],
        "boundary_variances": [
            names[j]
            for j in range(dimension)
            if bounds[j, 0] < bounds[j, 1]
            and min(om[j, j] - bounds[j, 0], bounds[j, 1] - om[j, j])
            <= 1e-6 * (bounds[j, 1] - bounds[j, 0])
        ],
        "identity": {
            **identity(
                model,
                {
                    "subjects": subjects,
                    "parameters": parameters,
                    "omega": omega,
                    "error": error,
                    "covariates": controls,
                    "seed": seed,
                    "covariance": covariance,
                    "variance_bounds": np.asarray(variance_bounds).tolist(),
                    "max_iterations": max_iterations,
                    "burn_in": burn_in,
                    "sweeps": sweeps,
                    "tolerance": tolerance,
                    "max_simulations": max_simulations,
                },
            ),
            "native": native_identity,
        },
    }
