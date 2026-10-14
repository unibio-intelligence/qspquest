"""Serializable fit diagnostics and seeded population predictive checks."""

from copy import deepcopy
import hashlib
import json
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
    observations,
    parameter_specs,
)


def fit_diagnostics(fit):
    """Return plotting rows, residual summaries, mixing and boundary warnings.

    Standardized residuals are conditional residuals, not CWRES or NPDE.
    Censored rows retain null residuals and their quantification limits.
    """
    np, _, _, _ = dependencies()
    rows = fit.get("fitted_observations")
    if not isinstance(rows, list) or not rows:
        fail("fit has no complete fitted observation rows")
    residuals = np.asarray(
        [
            r["standardized_residual"]
            for r in rows
            if r.get("standardized_residual") is not None
        ],
        dtype=float,
    )
    if not np.all(np.isfinite(residuals)):
        fail("fit residuals are not finite")
    warnings = []
    if fit.get("status") != "converged":
        warnings.append(f"fit status is {fit.get('status')}")
    if fit.get("boundary_parameters"):
        warnings.append("one or more structural estimates are at search bounds")
    if any(
        fit.get(k)
        for k in (
            "boundary_residual_errors",
            "boundary_covariate_coefficients",
            "boundary_variances",
        )
    ):
        warnings.append(
            "one or more residual, covariate or variance estimates are at search bounds"
        )
    if fit.get("objective_kind") == "saem_complete_data_surrogate":
        warnings.append(
            "complete-data surrogate cannot be used for marginal-likelihood model comparison"
        )
    return {
        "schema": "qspquest.fit-diagnostics/v0.1",
        "observations": deepcopy(rows),
        "residual_kind": "conditional_standardized_residual",
        "residual_summary": {
            "count": len(residuals),
            "mean": float(residuals.mean()) if len(residuals) else None,
            "sd": float(residuals.std(ddof=1)) if len(residuals) > 1 else None,
        },
        "censored_rows": sum(r.get("censoring") == "left" for r in rows),
        "parameter_trace": deepcopy(fit.get("trace", [])),
        "population_diagnostics": deepcopy(fit.get("diagnostics")),
        "warnings": warnings,
        "identity": deepcopy(fit.get("identity")),
    }


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
    Workers (1..8, default 1) preserve ordered random streams and records.
    Failed replicates remain in the denominator and suppress intervals.
    """
    np, _, _, _ = dependencies()
    integer(replicates, "replicates", 5, 2000)
    integer(seed, "seed", 0, 2**64 - 1)
    integer(workers, "workers", 1, 8)
    if not isinstance(subjects, list) or not 1 <= len(subjects) <= 128:
        fail("VPC requires 1..128 subjects")
    if not 0 < finite(confidence, "confidence") < 1:
        fail("confidence must be in (0, 1)")
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
    specs = {
        name: {
            "initial": q["value"],
            "lower": max(q["value"] * 0.001, 1e-15),
            "upper": q["value"] * 1000,
            "unit": q["unit"],
        }
        for name, q in parameters.items()
    }
    specs = parameter_specs(specs)
    names = list(parameters) if random_effects is None else list(random_effects)
    if (
        not 1 <= len(names) <= 8
        or len(set(names)) != len(names)
        or any(k not in specs for k in names)
    ):
        fail("VPC random effects must select 1..8 distinct positive parameters")
    om = np.asarray(omega, dtype=float)
    if (
        om.shape != (len(names), len(names))
        or not np.all(np.isfinite(om))
        or not np.allclose(om, om.T, atol=1e-12, rtol=0)
    ):
        fail("VPC Omega must be finite and symmetric")
    try:
        cholesky = np.linalg.cholesky(om)
    except np.linalg.LinAlgError:
        fail("VPC Omega must be positive definite")
    e = error_spec(error)
    if any(isinstance(e[k], dict) for k in ("additive", "proportional")):
        fail("VPC requires fitted or fixed numeric residual SDs")
    e = initial_error(e)
    if len(stratify) > 2 or not all(isinstance(k, str) for k in stratify):
        fail("stratify supports at most two subject covariates")
    budget, rng = Budget(max_simulations, control), np.random.default_rng(seed)
    if len(subjects) * (replicates + 1) > budget.maximum:
        fail("VPC max_simulations must cover one baseline and every requested replicate per subject", "budget")
    baseline, y_values, time_values, observed_censored, limits, labels = (
        [],
        [],
        [],
        [],
        [],
        [],
    )
    typical, lengths, units, native_identity = [], [], set(), None
    for subject in subjects:
        values = {name: q["value"] for name, q in parameters.items()}
        for covariate_control in covariates:
            name, key = covariate_control["parameter"], covariate_control["covariate"]
            if name not in values:
                fail("VPC covariate parameter is unknown")
            value = finite(
                subject.get("covariates", {}).get(key), "VPC subject covariate"
            )
            values[name] *= math.exp(
                covariate_control["estimate"] * (value - covariate_control["reference"])
            )
        typical.append(values)
        result = budget.run(model, bind(subject["run"], specs, values))
        native_identity = result["identity"]
        y, pred, cens = observations(result, subject["observations"])
        label_values = [subject.get("covariates", {}).get(k) for k in stratify]
        if any(v is None for v in label_values):
            fail("missing VPC stratification covariate")
        label = repr(tuple(label_values))
        baseline.extend(pred.tolist())
        y_values.extend(y.tolist())
        observed_censored.extend(cens.tolist())
        lengths.append(len(y))
        for observation in subject["observations"]:
            units.add(
                (
                    observation["output"],
                    observation["value"]["unit"],
                    result["time_unit"],
                )
            )
            time_values.append(result["times"][observation["row"]])
            labels.append(label)
            limit = observation.get("lloq")
            if limit is None and observation.get("censoring") == "left":
                limit = observation["value"]
            if limit is not None:
                if limit.get("unit") != observation["value"]["unit"]:
                    fail("LLOQ unit differs from observed output")
                limit_value = finite(limit.get("value"), "LLOQ")
                if (
                    observation.get("censoring") == "left"
                    and limit_value != observation["value"]["value"]
                ):
                    fail("censored observation value and LLOQ must name the same limit")
                limits.append(limit_value)
            else:
                limits.append(float("-inf"))
    if len(units) != 1:
        fail("one VPC requires one output, output unit and time unit")
    t, baseline, y, cens, limits = map(
        np.asarray, (time_values, baseline, y_values, observed_censored, limits)
    )
    if not np.all(np.isfinite(baseline)):
        fail("VPC baseline predictions must be finite", "domain")
    if len(y) * replicates > 2000000 or len(set(labels)) > 32:
        fail("VPC exceeds bounded storage or stratification limits")
    if bins is None:
        edges = np.linspace(t.min(), t.max(), min(6, len(set(time_values))) + 1)
        if t.min() == t.max():
            fail("VPC needs a nonzero time range")
    else:
        edges = np.asarray(bins, dtype=float)
    if (
        edges.ndim != 1
        or not 2 <= len(edges) <= 101
        or not np.all(np.isfinite(edges))
        or np.any(np.diff(edges) <= 0)
        or edges[0] > t.min()
        or edges[-1] < t.max()
    ):
        fail("VPC bins must be increasing and cover all observations")
    groups = np.minimum(np.searchsorted(edges, t, side="right") - 1, len(edges) - 2)
    cells = [
        (label, b, np.flatnonzero((np.asarray(labels) == label) & (groups == b)))
        for label in sorted(set(labels))
        for b in range(len(edges) - 1)
    ]
    correction = np.ones(len(y))
    if prediction_corrected:
        if np.any(baseline <= 0):
            fail("prediction correction requires positive population predictions")
        for _, _, indices in cells:
            if len(indices):
                correction[indices] = np.median(baseline[indices]) / baseline[indices]
    simulations, censor_simulations, replicate_records = [], [], []
    retained_rng = deepcopy(rng.bit_generator.state)
    if control is not None:
        from .control import restore_budget

        context = identity(
            model,
            {
                "subjects": subjects,
                "parameters": parameters,
                "omega": omega,
                "error": error,
                "random_effects": names,
                "covariates": list(covariates),
                "replicates": replicates,
                "bins": edges.tolist(),
                "percentiles": p.tolist(),
                "confidence": confidence,
                "prediction_corrected": prediction_corrected,
                "stratify": list(stratify),
                "seed": seed,
                "max_simulations": max_simulations,
            },
        )
        saved = control.start("vpc", context, native_identity)
        if saved is not None:
            required = {"simulations", "draws", "censored", "replicate_records", "rng"}
            if (not required.issubset(saved)
                    or any(not isinstance(saved[key], list) for key in ("draws", "censored", "replicate_records"))):
                fail("VPC checkpoint state is incomplete", "checkpoint")
            restore_budget(budget, saved["simulations"])
            try:
                simulations = [None if v is None else np.asarray(v, dtype=float) for v in saved["draws"]]
                censor_simulations = [None if v is None else np.asarray(v) for v in saved["censored"]]
            except (TypeError, ValueError):
                fail("VPC checkpoint draw arrays are invalid", "checkpoint")
            replicate_records = saved["replicate_records"]
            if (not len(simulations) == len(censor_simulations) == len(replicate_records)
                    or len(simulations) > replicates):
                fail("VPC checkpoint replicate counts differ", "checkpoint")
            for i, (draw, censored, record) in enumerate(zip(simulations, censor_simulations, replicate_records)):
                if (not isinstance(record, dict) or record.get("replicate") != i or record.get("status") not in ("complete", "failed")
                        or not isinstance(record.get("stream_id"), str)
                        or not record["stream_id"].startswith("sha256:")
                        or len(record["stream_id"]) != 71
                        or any(c not in "0123456789abcdef" for c in record["stream_id"][7:])
                        or type(record.get("simulations")) is not int
                        or not 0 <= record["simulations"] <= len(subjects)
                        or (draw is None) != (record["status"] == "failed")
                        or (censored is None) != (draw is None)
                        or (draw is not None and (draw.shape != y.shape or censored.shape != y.shape or censored.dtype.kind != "b"
                                                  or not np.all(np.isfinite(draw))))):
                    fail("VPC checkpoint replicate shape or status differs", "checkpoint")
            try:
                rng.bit_generator.state = saved["rng"]
            except (TypeError, ValueError, KeyError, OverflowError):
                fail("VPC checkpoint random state is invalid", "checkpoint")
            retained_rng = deepcopy(saved["rng"])

    def work_items():
        # Assign the same legacy random stream to every replicate before native
        # execution. Prefetch and completion order never affect random draws.
        for replicate in range(len(simulations), replicates):
            start = deepcopy(rng.bit_generator.state)
            noise = [(rng.normal(size=len(names)), rng.normal(size=length)) for length in lengths]
            stream = "sha256:" + hashlib.sha256(json.dumps(start, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
            yield replicate, noise, stream, deepcopy(rng.bit_generator.state)

    def execute(item):
        replicate, noise, stream, after = item
        record = {"replicate": replicate, "stream_id": stream, "status": "complete", "simulations": 0}
        simulated = []
        try:
            for i, subject in enumerate(subjects):
                values = typical[i].copy()
                effects, residual = noise[i]
                for name, eta in zip(names, cholesky @ effects):
                    values[name] *= math.exp(eta)
                request = bind(subject["run"], specs, values)
                record["simulations"] += 1
                result = budget.run(model, request)
                _, pred, _ = observations(result, subject["observations"])
                if len(pred) != lengths[i] or not np.all(np.isfinite(pred)):
                    fail("VPC prediction shape or finiteness differs from the admitted design", "domain")
                with np.errstate(over="ignore", invalid="ignore"):
                    if e["model"] == "lognormal":
                        if np.any(pred <= 0):
                            fail("lognormal VPC needs positive predictions", "domain")
                        draw = pred * np.exp(e["proportional"] * residual)
                    else:
                        sd = np.sqrt(e["additive"] ** 2 + (e["proportional"] * pred) ** 2)
                        draw = pred + sd * residual
                if not np.all(np.isfinite(draw)):
                    fail("VPC simulated observations must be finite", "domain")
                simulated.extend(draw.tolist())
            simulated = np.asarray(simulated)
            corrected = simulated * correction
            if not np.all(np.isfinite(corrected)):
                fail("VPC prediction-corrected observations must be finite", "domain")
            return corrected, simulated < limits, record, after
        except (ValueError, OverflowError) as exception:
            code = getattr(exception, "code", "domain" if isinstance(exception, OverflowError) else "invalid_input")
            if code in ("budget", "cancelled"):
                raise
            record.update(status="failed", code=code, message=str(exception))
            return None, None, record, after

    def checkpoint_state():
        return {"draws": [None if v is None else v.tolist() for v in simulations],
                "censored": [None if v is None else v.tolist() for v in censor_simulations],
                "replicate_records": replicate_records, "rng": retained_rng,
                "simulations": budget.used}

    from ._work import ordered_work
    try:
        with ordered_work(work_items(), execute, workers, control) as results:
            for draw, censored, record, after in results:
                simulations.append(draw)
                censor_simulations.append(censored)
                replicate_records.append(record)
                retained_rng = after
                if control is not None:
                    control.boundary(len(simulations), replicates, budget, checkpoint_state(),
                                     final=len(simulations) == replicates)
    except ValueError as exception:
        # Drained workers have finished charging calls. Preserve their cost but
        # resume the last complete ordered prefix; unfinished rows are replayed.
        if getattr(exception, "code", None) in ("cancelled", "budget") and control is not None:
            control._retain_checkpoint(checkpoint_state())
        raise
    successful = sum(v is not None for v in simulations)
    failed = replicates - successful
    intervals_available = failed == 0
    draws = np.asarray([v for v in simulations if v is not None])
    blq_draws = np.asarray([v for v in censor_simulations if v is not None], dtype=bool)
    alpha = (1 - confidence) / 2
    records = []
    for label, b, indices in cells:
        if not len(indices):
            continue
        observed = y[indices][~cens[indices]] * correction[indices][~cens[indices]]
        quantiles = []
        for replicate in range(successful):
            values = draws[replicate, indices][~blq_draws[replicate, indices]]
            quantiles.append(
                np.quantile(values, p) if len(values) else np.full(len(p), np.nan)
            )
        quantiles = np.asarray(quantiles).reshape(successful, len(p))
        summaries = []
        for j, percentile in enumerate(p):
            finite_values = quantiles[:, j][np.isfinite(quantiles[:, j])]
            summaries.append(
                {
                    "percentile": float(percentile),
                    "observed": (
                        float(np.quantile(observed, percentile))
                        if len(observed)
                        else None
                    ),
                    "simulated": (
                        float(np.median(finite_values)) if len(finite_values) else None
                    ),
                    "confidence_interval": (
                        np.quantile(finite_values, [alpha, 1 - alpha]).tolist()
                        if intervals_available and len(finite_values)
                        else None
                    ),
                    "quantifiable_replicates": len(finite_values),
                }
            )
        fractions = blq_draws[:, indices].mean(axis=1) if successful else np.asarray([])
        records.append(
            {
                "stratum": label,
                "start": float(edges[b]),
                "end": float(edges[b + 1]),
                "observations": len(indices),
                "quantiles": summaries,
                "observed_blq_fraction": float(cens[indices].mean()),
                "simulated_blq_fraction": float(fractions.mean()) if len(fractions) else None,
                "blq_confidence_interval": np.quantile(
                    fractions, [alpha, 1 - alpha]
                ).tolist() if intervals_available and len(fractions) else None,
            }
        )
    output, unit, time_unit = next(iter(units))
    return {
        "schema": "qspquest.vpc/v0.1",
        "seed": seed,
        "replicates": replicates,
        "status": "complete" if failed == 0 else "incomplete",
        "successful_replicates": successful,
        "failed_replicates": failed,
        "replicate_records": replicate_records,
        "execution": {"workers": workers, "maximum_in_flight": min(workers, replicates),
                      "ordering": "replicate_index", "random_stream": "legacy_ordered_normal_draws/v1"},
        "prediction_corrected": bool(prediction_corrected),
        "output": output,
        "unit": unit,
        "time_unit": time_unit,
        "confidence": confidence,
        "bins": records,
        "lloq_coverage": int(np.isfinite(limits).sum()),
        "total_observations": len(y),
        "simulations": budget.used,
        "identity": {
            **identity(
                model,
                {
                    "subjects": subjects,
                    "parameters": parameters,
                    "omega": omega,
                    "error": error,
                    "random_effects": names,
                    "covariates": list(covariates),
                    "replicates": replicates,
                    "bins": edges.tolist(),
                    "percentiles": p.tolist(),
                    "confidence": confidence,
                    "prediction_corrected": prediction_corrected,
                    "stratify": list(stratify),
                    "seed": seed,
                    "max_simulations": max_simulations,
                },
            ),
            "native": native_identity,
        },
    }
