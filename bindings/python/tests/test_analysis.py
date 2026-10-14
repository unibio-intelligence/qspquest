"""Independent scientific checks for native-backed analysis workflows."""

from copy import deepcopy
import json
import math
from pathlib import Path
import numpy as np
import pytest
from scipy import special
from scipy.integrate import solve_ivp
from scipy.linalg import expm
from qspquest import (
    CompiledModel,
    QSPQuestError,
    exposure_summary,
    fit_diagnostics,
    write_analysis_report,
)
from qspquest._analysis import likelihood

EXAMPLES = Path(__file__).resolve().parents[3] / "examples" / "synthetic"


def test_saem_stability_window_does_not_hide_drift_as_gain_shrinks():
    from qspquest.population import _stability_history

    trace, clock = [], 0.0
    for iteration in range(1, 10001):
        clock += iteration ** -0.7
        trace.append({"iteration": iteration, "coordinates": [0.02 * clock]})
    old = np.asarray([row["coordinates"] for row in trace[-50:]])
    assert np.max(abs(old[:20].mean(axis=0) - old[-20:].mean(axis=0))) < 0.001
    history = np.asarray(_stability_history(trace, 0))
    end_count = len(history) * 2 // 5
    assert (
        np.max(abs(history[:end_count].mean(axis=0) - history[-end_count:].mean(axis=0)))
        > 0.001
    )

    # A short post-burn history cannot borrow its span from burn-in records.
    assert _stability_history(trace[:70], 30) is None
    flat = [{"iteration": row["iteration"], "coordinates": [1.0]} for row in trace]
    assert _stability_history(flat, 0) is not None


def model(name="absorption"):
    return CompiledModel((EXAMPLES / f"{name}.qsp").read_text())


def request(
    times=(0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 24.0),
    *,
    target="depot",
    amount=320.0,
    time_unit="h",
    solver="analytic_linear_pk",
):
    return {
        "schema": "qspquest.run/v0.1",
        "solver": solver,
        "budgets": {
            "solver_callbacks": 1000000,
            "events": 1000,
            "output_values": 100000,
        },
        "regimen": {
            "end": {"value": max(times), "unit": time_unit},
            "samples": [],
            "observations": [
                {"time": {"value": t, "unit": time_unit}, "side": "post"} for t in times
            ],
            "administrations": [
                {
                    "target": target,
                    "time": {"value": 0.0, "unit": time_unit},
                    "amount": {"value": amount, "unit": "mg"},
                    "delivery": {"kind": "bolus"},
                    "bioavailability": 1.0,
                }
            ],
            "rtol": 1e-9,
            "atol": 1e-11,
        },
    }


def parameters():
    return {
        "cl": {"initial": 2.0, "lower": 0.1, "upper": 5.0, "unit": "L/h"},
        "vc": {"initial": 28.0, "lower": 0.5, "upper": 50.0, "unit": "L"},
        "ka": {"initial": 1.0, "lower": 0.2, "upper": 3.0, "unit": "1/h"},
    }


def observations(result):
    return [
        {"row": i, "output": "cp", "value": {"value": v, "unit": "mg/L"}}
        for i, v in enumerate(result["outputs"][0]["values"])
    ]


def test_synthetic_equations_match_independent_solutions():
    times = np.asarray([0.1, 0.25, 1, 2, 8, 24])
    oral = model().run(request(times))
    cl, vc, ka = 0.8, 6.0, 1.2
    reference = (
        320
        / vc
        * ka
        / (ka - cl / vc)
        * (np.exp(-cl / vc * times) - np.exp(-ka * times))
    )
    np.testing.assert_allclose(oral["outputs"][0]["values"], reference, rtol=1e-10)
    two = model("distribution").run(request(times, target="central"))
    cl, vc, vp, q = 0.8, 6.0, 12.0, 1.1
    matrix = np.array([[-(cl + q) / vc, q / vp], [q / vc, -q / vp]])
    reference = [(expm(matrix * t) @ np.array([320, 0]))[0] / vc for t in times]
    np.testing.assert_allclose(two["outputs"][0]["values"], reference, rtol=1e-10)
    mm = model("saturable").run(
        request(times, target="central", amount=10, time_unit="d", solver="diffsol_bdf")
    )
    solution = solve_ivp(
        lambda t, a: [-0.3 / 4 * a[0] - 0.2 * (a[0] / 4) / (1.6 + a[0] / 4)],
        (0, times[-1]),
        [10],
        t_eval=times,
        method="DOP853",
        rtol=1e-12,
        atol=1e-13,
    )
    np.testing.assert_allclose(
        mm["outputs"][0]["values"], solution.y[0] / 4, rtol=2e-7, atol=1e-8
    )


def test_combined_and_censored_likelihood_matches_independent_formula():
    y, p, cens = np.array([1.0, 0.2]), np.array([1.2, 0.7]), np.array([False, True])
    error = {"model": "normal", "additive": 0.1, "proportional": 0.2}
    sd = np.sqrt(0.1**2 + (0.2 * p) ** 2)
    expected = (
        0.5 * ((y[0] - p[0]) / sd[0]) ** 2
        + np.log(sd[0])
        + 0.5 * np.log(2 * np.pi)
        - special.log_ndtr((y[1] - p[1]) / sd[1])
    )
    assert likelihood(y, p, cens, error)[0] == pytest.approx(expected, abs=1e-12)
    assert math.isfinite(
        likelihood(
            np.array([-50.0]),
            np.array([1.0]),
            np.array([True]),
            {"model": "normal", "additive": 0.1, "proportional": 0},
        )[0]
    )


def test_deterministic_recovery_and_budget():
    m, run = model(), request()
    obs = observations(m.run(run))
    fit = m.fit_data(run, obs, parameters(), {"additive": 0.1})
    assert fit["status"] == "converged"
    assert fit["objective"] < fit["initial_objective"]
    for name, truth in zip(("cl", "vc", "ka"), (0.8, 6.0, 1.2)):
        assert fit["parameters"][name]["value"] == pytest.approx(truth, rel=1e-5)
    limited = m.fit_data(run, obs, parameters(), {"additive": 0.1}, max_simulations=2)
    assert limited["status"] == "evaluation_limit" and limited["simulations"] == 2
    with pytest.raises(QSPQuestError, match="units"):
        bad = deepcopy(obs)
        bad[0]["value"]["unit"] = "ng/mL"
        m.fit_data(run, bad, parameters(), {"additive": 0.1})


def test_moving_lag_derivative_and_fit():
    m, run = model(), request()
    run["regimen"]["administrations"][0]["lag"] = {"value": 0.1, "unit": "h"}
    specs = {
        "lag": {
            "initial": 0.1,
            "lower": 0.0,
            "upper": 0.2,
            "unit": "h",
            "path": ["regimen", "administrations", 0, "lag"],
        }
    }
    derivative = m.event_sensitivities(run, specs)
    cl, vc, ka = 0.8, 6.0, 1.2
    t = np.asarray(derivative["times"]) - 0.1
    analytic = (
        320
        / vc
        * ka
        / (ka - cl / vc)
        * (cl / vc * np.exp(-cl / vc * t) - ka * np.exp(-ka * t))
    )
    np.testing.assert_allclose(
        derivative["derivatives"]["lag"]["cp"]["values"], analytic, rtol=1e-6, atol=1e-6
    )
    truth_run = deepcopy(run)
    truth_run["regimen"]["administrations"][0]["lag"]["value"] = 0.15
    fit = m.fit_data(run, observations(m.run(truth_run)), specs, {"additive": 0.1})
    assert fit["status"] == "converged"
    assert fit["parameters"]["lag"]["value"] == pytest.approx(0.15, abs=1e-6)


def test_discontinuous_event_derivative_rejected():
    m, run = model("distribution"), request((0.1, 0.25, 1, 2), target="central")
    run["regimen"]["administrations"][0]["time"] = {"value": 0.1, "unit": "h"}
    specs = {
        "time": {
            "initial": 0.1,
            "lower": 0.01,
            "upper": 0.2,
            "unit": "h",
            "path": ["regimen", "administrations", 0, "time"],
        }
    }
    with pytest.raises(QSPQuestError, match="discontinuous|observation grid"):
        m.event_sensitivities(run, specs)


def test_sided_exposure_and_threshold_have_analytical_results():
    result = {
        "times": [0.0, 1.0, 1.0, 2.0],
        "sides": ["post", "pre", "post", "pre"],
        "time_unit": "h",
        "outputs": [{"name": "cp", "unit": "mg/L", "values": [0.0, 0.0, 4.0, 2.0]}],
    }
    summary = exposure_summary(result, "cp", threshold=3)
    assert summary["auc"] == 3
    assert summary["cmax"] == 4 and summary["tmax"] == 1
    assert summary["time_above_threshold"]["duration"] == 0.5
    bad = deepcopy(result)
    bad["sides"][1] = "post"
    with pytest.raises(QSPQuestError, match="pre then post"):
        exposure_summary(bad, "cp")


def test_population_comparison_uses_paired_subjects_and_budget():
    m, run = model(), request(np.linspace(0.001, 24, 200))
    higher = deepcopy(run)
    higher["regimen"]["administrations"][0]["amount"]["value"] *= 2
    params = {
        "cl": {"value": math.e, "unit": "L/h"},
        "vc": {"value": math.exp(3.45), "unit": "L"},
        "ka": {"value": math.exp(0.45), "unit": "1/h"},
    }
    comparison = m.compare_regimens(
        {"base": run, "double": higher}, params, np.eye(3).tolist(), "cp", subjects=4
    )
    for a, b in zip(
        comparison["regimens"]["base"]["subject_metrics"],
        comparison["regimens"]["double"]["subject_metrics"],
    ):
        assert b["auc"] == pytest.approx(2 * a["auc"], rel=1e-10)
    with pytest.raises(QSPQuestError, match="budget"):
        m.compare_regimens(
            {"base": run},
            params,
            np.eye(3).tolist(),
            "cp",
            subjects=4,
            max_simulations=3,
        )


def test_profile_matches_known_information():
    # For fixed V/Ka, CL profile independently compares scalar likelihood.
    m, run = model(), request()
    obs = observations(m.run(run))
    params = {"cl": {"initial": 2.0, "lower": 0.1, "upper": 5.0, "unit": "L/h"}}
    result = m.profile_likelihood(
        run, obs, params, {"additive": 0.1}, "cl", np.linspace(0.6, 1.0, 21).tolist()
    )
    assert result["status"] == "complete"
    assert result["interval"][0] < 0.8 < result["interval"][1]
    for row in result["profile"]:
        trial = deepcopy(run)
        trial["parameters"] = {"cl": {"value": row["value"], "unit": "L/h"}}
        p = np.array(m.run(trial)["outputs"][0]["values"])
        y = np.array([o["value"]["value"] for o in obs])
        expected = 0.5 * np.sum(((y - p) / 0.1) ** 2) + len(y) * math.log(
            0.1 * math.sqrt(2 * math.pi)
        )
        assert row["objective"] == pytest.approx(expected, abs=1e-8)


def cohort(count=8):
    # Known oral-PK equations, with explicitly generated recovery observations.
    m, run, rng = model(), request(), np.random.default_rng(18)
    subjects = []
    truth = {"cl": 0.8, "vc": 6.0, "ka": 1.2}
    eta = rng.normal(0, 0.12, (count, 3))
    eta -= eta.mean(axis=0)
    for i in range(count):
        weight = (i - (count - 1) / 2) / count
        r = deepcopy(run)
        r["parameters"] = {
            k: {
                "value": v * math.exp(eta[i, j] + (0.3 * weight if k == "cl" else 0)),
                "unit": parameters()[k]["unit"],
            }
            for j, (k, v) in enumerate(truth.items())
        }
        obs = observations(m.run(r))
        for o in obs:
            o["value"]["value"] += rng.normal(0, 0.04)
        subjects.append(
            {
                "id": str(i),
                "run": run,
                "observations": obs,
                "covariates": {"weight_centered": weight},
            }
        )
    return m, subjects


def test_general_saem_three_effects_joint_covariate_error_and_repeatability():
    m, subjects = cohort()
    cov = [
        {
            "parameter": "cl",
            "covariate": "weight_centered",
            "reference": 0.0,
            "initial": 0.0,
            "lower": -1.0,
            "upper": 1.0,
        }
    ]
    error = {"additive": {"initial": 0.1, "lower": 0.01, "upper": 0.3}}
    options = dict(
        covariates=cov,
        covariance="diagonal",
        max_iterations=250,
        burn_in=60,
        sweeps=2,
        seed=22,
        tolerance=0.03,
    )
    fit = m.fit_population(
        subjects, parameters(), (np.eye(3) * 0.04).tolist(), error, **options
    )
    assert fit["status"] in ("converged", "iteration_limit", "mixing_failed")
    assert fit["objective_kind"] == "saem_complete_data_surrogate"
    assert len(fit["fitted_observations"]) == sum(
        len(s["observations"]) for s in subjects
    )
    for name, truth in zip(("cl", "vc", "ka"), (0.8, 6.0, 1.2)):
        assert fit["parameters"][name]["value"] == pytest.approx(truth, rel=0.15)
    assert fit["error"]["additive"] < 0.1
    assert fit["covariate_effects"][0]["estimate"] > 0
    again = m.fit_population(
        subjects, parameters(), (np.eye(3) * 0.04).tolist(), error, **options
    )
    assert fit["parameters"] == again["parameters"] and fit["omega"] == again["omega"]
    assert fit["trace"] == again["trace"]
    limited = m.fit_population(
        subjects,
        parameters(),
        (np.eye(3) * 0.04).tolist(),
        error,
        max_iterations=60,
        burn_in=20,
        max_simulations=10,
    )
    assert limited["status"] == "evaluation_limit" and limited["simulations"] == 10


def test_vpc_censoring_reproducibility_and_report(tmp_path):
    m, subjects = cohort(6)
    for s in subjects:
        for o in s["observations"]:
            o["lloq"] = {"value": 2.0, "unit": "mg/L"}
            if o["value"]["value"] < 2:
                o["value"]["value"], o["censoring"] = 2.0, "left"
    params = {
        "cl": {"value": math.e, "unit": "L/h"},
        "vc": {"value": math.exp(3.45), "unit": "L"},
        "ka": {"value": math.exp(0.45), "unit": "1/h"},
    }
    options = dict(replicates=10, prediction_corrected=True, seed=7)
    first = m.visual_predictive_check(
        subjects, params, (np.eye(3) * 0.02).tolist(), {"additive": 0.04}, **options
    )
    second = m.visual_predictive_check(
        subjects, params, (np.eye(3) * 0.02).tolist(), {"additive": 0.04}, **options
    )
    assert first == second and first["lloq_coverage"] == first["total_observations"]
    assert any(b["observed_blq_fraction"] > 0 for b in first["bins"])
    assert all(0 <= b["simulated_blq_fraction"] <= 1 for b in first["bins"])
    report = tmp_path / "report.html"
    write_analysis_report(report, title="<script>unsafe</script>", vpc=first)
    assert "<script>unsafe</script>" not in report.read_text()
    assert "<svg" in report.read_text()
    json.dumps(first, allow_nan=False)


def test_bootstrap_accounts_for_all_failures_and_covariate_rank():
    m, subjects = cohort(6)
    result = m.bootstrap_population(
        subjects,
        parameters(),
        (np.eye(3) * 0.04).tolist(),
        {"additive": 0.04},
        replicates=3,
        max_simulations=7,
        max_iterations=60,
        burn_in=20,
    )
    assert len(result["replicates"]) == 3
    assert result["successful_replicates"] == 0 and result["failed_replicates"] == 3
    assert result["intervals"] is None and result["simulations"] == 7
    assert len({r["seed"] for r in result["replicates"]}) == 3
    cov = [
        {
            "parameter": "cl",
            "covariate": "constant",
            "reference": 0.0,
            "initial": 0.0,
            "lower": -1.0,
            "upper": 1.0,
        }
    ]
    for s in subjects:
        s["covariates"]["constant"] = 1.0
    with pytest.raises(QSPQuestError, match="rank deficient"):
        m.fit_population(
            subjects,
            parameters(),
            (np.eye(3) * 0.04).tolist(),
            {"additive": 0.04},
            covariates=cov,
        )


def test_successful_subject_bootstrap_and_full_covariance_recovery():
    m, run, rng = model(), request(), np.random.default_rng(31)
    subjects = []
    eta = rng.normal(0, 0.15, 8)
    eta -= eta.mean()
    for i in range(8):
        trial = deepcopy(run)
        trial["parameters"] = {"ka": {"value": math.exp(0.45 + eta[i]), "unit": "1/h"}}
        obs = observations(m.run(trial))
        for o in obs:
            o["value"]["value"] += rng.normal(0, 0.04)
        subjects.append({"id": str(i), "run": run, "observations": obs})
    params = {"ka": {"initial": 1.5, "lower": 0.2, "upper": 3.0, "unit": "1/h"}}
    result = m.bootstrap_population(
        subjects,
        params,
        [[0.04]],
        {"additive": 0.04},
        replicates=5,
        confidence=0.8,
        minimum_success_fraction=0.6,
        max_iterations=250,
        burn_in=50,
        sweeps=3,
        tolerance=0.03,
        seed=44,
    )
    assert result["status"] == "complete"
    assert result["successful_replicates"] >= 3
    assert (
        result["intervals"]["ka"]["bounds"][0] < result["intervals"]["ka"]["bounds"][1]
    )
    assert result["nuisance_intervals"]["omega"] is not None
    assert all(len(r["source_indices"]) == 8 for r in result["replicates"])
    m, subjects = cohort(8)
    full = m.fit_population(
        subjects,
        parameters(),
        (np.eye(3) * 0.04).tolist(),
        {"additive": 0.04},
        covariance="full",
        max_iterations=250,
        burn_in=60,
        sweeps=3,
        tolerance=0.03,
        seed=22,
    )
    assert full["status"] == "converged"
    assert np.all(np.linalg.eigvalsh(full["omega"]) > 0)
    for name, truth in zip(("cl", "vc", "ka"), (0.8, 6.0, 1.2)):
        assert full["parameters"][name]["value"] == pytest.approx(truth, rel=0.15)


def test_two_effect_covariate_combined_error_and_blq():
    m, subjects = cohort(8)
    for s in subjects:
        for o in s["observations"]:
            if o["value"]["value"] < 2.0:
                o["value"]["value"] = 2.0
                o["censoring"] = "left"
    selected = {k: v for k, v in parameters().items() if k != "ka"}
    cov = [
        {
            "parameter": "cl",
            "covariate": "weight_centered",
            "reference": 0.0,
            "initial": 0.0,
            "lower": -1.0,
            "upper": 1.0,
        }
    ]
    error = {
        "additive": {"initial": 0.1, "lower": 0.02, "upper": 0.3},
        "proportional": {"initial": 0.02, "lower": 0.001, "upper": 0.2},
    }
    fit = m.fit_population(
        subjects,
        selected,
        (np.eye(2) * 0.04).tolist(),
        error,
        covariates=cov,
        covariance="diagonal",
        max_iterations=160,
        burn_in=40,
        sweeps=3,
        seed=24,
        tolerance=0.03,
    )
    assert fit["status"] != "evaluation_limit"
    assert all(math.isfinite(v) for v in np.asarray(fit["omega"]).flat)
    assert 0.02 <= fit["error"]["additive"] <= 0.3
    assert 0.001 <= fit["error"]["proportional"] <= 0.2
    assert fit["parameters"]["cl"]["value"] == pytest.approx(0.8, rel=0.2)
    censored = [r for r in fit["fitted_observations"] if r["censoring"] == "left"]
    assert censored and all(r["residual"] is None for r in censored)
