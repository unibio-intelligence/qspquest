"""Checked numerical derivatives of complete native regimens."""

from ._analysis import (
    Budget,
    bind,
    columns,
    dependencies,
    fail,
    finite,
    identity,
    parameter_specs,
)


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
    np, _, _, _ = dependencies()
    specs = parameter_specs(parameters)
    if (
        not 0 < finite(relative_step, "relative_step") < 0.1
        or not 0 < finite(relative_tolerance, "relative_tolerance") < 1
        or finite(absolute_tolerance, "absolute_tolerance") <= 0
    ):
        fail("derivative steps and tolerances must be finite positive values")
    budget = Budget(max_simulations)
    values = {k: s["initial"] for k, s in specs.items()}
    baseline = budget.run(model, bind(run, specs, values))
    reference_grid = (baseline["times"], baseline["sides"])
    reference = columns(baseline)
    derivatives, errors = {}, {}
    for name, spec in specs.items():
        center = spec["initial"]
        h = relative_step * max(abs(center), (spec["upper"] - spec["lower"]) * 0.1)
        if center - h <= spec["lower"] or center + h >= spec["upper"]:
            fail(
                "central derivatives require an interior parameter with room for both steps"
            )
        approximations = []
        for step in (h, h / 2):
            perturbed = []
            for sign in (-1, 1):
                trial = values.copy()
                trial[name] += sign * step
                result = budget.run(model, bind(run, specs, trial))
                if (result["times"], result["sides"]) != reference_grid:
                    fail(
                        "moving event changes the observation grid; use fixed observation times away from events",
                        "unsupported",
                    )
                data = columns(result)
                if set(data) != set(reference) or any(
                    data[k][1] != reference[k][1] for k in data
                ):
                    fail("perturbed trajectory changes output names or units")
                perturbed.append(data)
            approximations.append(
                {
                    k: (perturbed[1][k][0] - perturbed[0][k][0]) / (2 * step)
                    for k in reference
                }
            )
        derivatives[name], errors[name] = {}, {}
        for output, (_, unit) in reference.items():
            coarse, fine = approximations[0][output], approximations[1][output]
            error = np.abs(fine - coarse)
            if not np.all(np.isfinite(fine)) or np.any(
                error > absolute_tolerance + relative_tolerance * np.abs(fine)
            ):
                fail(
                    f"derivative for {name}/{output} is discontinuous or unresolved at the requested grid",
                    "unsupported",
                )
            derivatives[name][output] = {
                "values": fine.tolist(),
                "unit": f"{unit} / {spec['unit']}",
            }
            errors[name][output] = error.tolist()
    return {
        "schema": "qspquest.event-sensitivities/v0.1",
        "method": "checked_central_difference",
        "times": baseline["times"],
        "sides": baseline["sides"],
        "time_unit": baseline["time_unit"],
        "derivatives": derivatives,
        "step_difference": errors,
        "simulations": budget.used,
        "identity": {
            **identity(
                model,
                {
                    "run": run,
                    "parameters": parameters,
                    "relative_step": relative_step,
                    "relative_tolerance": relative_tolerance,
                    "absolute_tolerance": absolute_tolerance,
                    "max_simulations": max_simulations,
                },
            ),
            "native": baseline["identity"],
        },
    }
