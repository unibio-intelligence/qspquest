"""Profile optima and nuisance fits checked against independent solutions."""

import math
import numpy as np
import pytest
from scipy.optimize import minimize_scalar
from qspquest import CompiledModel


def request(times, parameters, initial_states=None):
    result = {
        "schema": "qspquest.run/v0.1",
        "solver": "diffsol_bdf",
        "budgets": {"events": 100, "output_values": 1000, "solver_callbacks": 10000},
        "parameters": parameters,
        "regimen": {
            "start": {"value": 0, "unit": "h"},
            "end": {"value": 2, "unit": "h"},
            "administrations": [], "samples": [],
            "observations": [{"time": {"value": float(t), "unit": "h"}, "side": "post"} for t in times],
            "rtol": 1e-10, "atol": 1e-12,
        },
    }
    if initial_states is not None:
        result["initial_states"] = initial_states
    return result


@pytest.mark.parametrize("index", [59, 68])
def test_profile_gaussian_mean_and_fitted_sd(index):
    model = CompiledModel("""model gaussian_profile
time_unit = h
parameters
 theta = 2.718281828459045 [1] estimated
 decay = 0 [1/h] fixed
states
 anchor = 0 [1]
dynamics
 d/dt(anchor) = -decay * anchor
outputs
 z = log(theta) [1]
end
""")
    times = np.linspace(0, 2, 24)
    rng = np.random.Generator(np.random.PCG64(np.random.SeedSequence([20261026, 53, 1, index])))
    y = 1.4 + rng.normal(0, .25, 24)
    rows = [{"row": i, "output": "z", "value": {"value": float(v), "unit": "1"}} for i, v in enumerate(y)]
    result = model.profile_likelihood(
        request(times, {"theta": {"value": math.exp(1), "unit": "1"}}), rows,
        {"theta": {"initial": math.exp(1), "lower": math.exp(-.2), "upper": math.exp(3), "unit": "1"}},
        {"additive": {"initial": .2, "lower": .05, "upper": .8}},
        "theta", np.exp(np.linspace(.8, 2, 161)).tolist(),
    )
    assert result["status"] == "complete"
    assert abs(math.log(result["estimate"]) - y.mean()) <= 1e-5
    for row in result["profile"]:
        sd = np.clip(np.sqrt(np.mean((y - math.log(row["value"])) ** 2)), .05, .8)
        expected = .5 * np.sum(((y - math.log(row["value"])) / sd) ** 2) + len(y) * math.log(sd * math.sqrt(2 * math.pi))
        assert row["objective"] == pytest.approx(expected, abs=1e-5)


@pytest.mark.parametrize("nuisance,index,panel", [(False, 339, 2), (True, 0, 3), (True, 79, 3)])
def test_profile_pk_complete_and_matches_closed_form(nuisance, index, panel):
    model = CompiledModel("""model pk_profile
time_unit = h
parameters
 cl = 0.6 [L/h] estimated
 v = 4.5 [L] fixed
states
 a = 6 [mg]
dynamics
 d/dt(a) = -cl * a / v
outputs
 c = a / v [mg/L]
end
""")
    times = np.linspace(0, 2, 9)
    rng = np.random.Generator(np.random.PCG64(np.random.SeedSequence([20261026, 53, panel, index])))
    y = 6 / 4.5 * np.exp(-.6 * times / 4.5) + rng.normal(0, .02, 9)
    rows = [{"row": i, "output": "c", "value": {"value": float(v), "unit": "mg/L"}} for i, v in enumerate(y)]
    parameters = {"cl": {"initial": .5, "lower": .2, "upper": 1, "unit": "L/h"}}
    if nuisance:
        parameters["v"] = {"initial": 4, "lower": 2.5, "upper": 7, "unit": "L"}
    result = model.profile_likelihood(
        request(times, {"cl": {"value": .5, "unit": "L/h"}, "v": {"value": 4.5, "unit": "L"}}, {"a": {"value": 6, "unit": "mg"}}),
        rows, parameters, {"additive": .02}, "cl", np.linspace(.3, .9, 161).tolist(),
    )
    assert result["status"] == "complete"
    assert result["simulations"] <= 20000
    for row in result["profile"]:
        def objective(volume):
            prediction = 6 / volume * np.exp(-row["value"] * times / volume)
            return float(.5 * np.sum(((y - prediction) / .02) ** 2) + len(y) * math.log(.02 * math.sqrt(2 * math.pi)))
        reference = minimize_scalar(objective, bounds=(2.5, 7), method="bounded", options={"xatol": 1e-12}).fun if nuisance else objective(4.5)
        assert row["status"] == "converged"
        assert row["objective"] == pytest.approx(reference, abs=1e-5)
