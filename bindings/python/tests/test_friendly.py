"""Synthetic-model interface and interrupted/restarted scientific workflow checks."""

from copy import deepcopy
import json
from pathlib import Path
import sys
import numpy as np
import pytest
import qspquest as pf
from qspquest.doctor import check_environment

ROOT = Path(__file__).resolve().parents[3]
MODEL = ROOT / "examples/synthetic/absorption.qsp"


def cohort():
    model = pf.CompiledModel(MODEL.read_text())
    subjects = []
    for i, cl in enumerate([2.2, 2.5, 2.8, 3.1, 3.5, 3.8]):
        run = pf.run_request(
            end=12,
            times=[0.25, 0.5, 1, 2, 4, 8, 12],
            administrations=[pf.bolus("depot", 300, "mg")],
            solver="analytic_linear_pk",
            parameters={"cl": pf.quantity(cl, "L/h")},
        )
        result = model.run(run)
        values = result["outputs"][0]["values"]
        obs = [
            {
                "row": j,
                "output": "cp",
                "value": pf.quantity(v + 0.02 * np.sin(i + j), "mg/L"),
            }
            for j, v in enumerate(values)
        ]
        run.pop("parameters")
        subjects.append({"id": str(i), "run": run, "observations": obs})
    parameters = {"cl": {"initial": 3.0, "lower": 0.5, "upper": 8.0, "unit": "L/h"}}
    return model, subjects, parameters


def test_native_helpers_frames_status_and_doctor():
    pytest.importorskip("pandas", reason="DataFrame helpers require the notebook extra")
    m, subjects, p = cohort()
    r = m.run(subjects[0]["run"])
    assert isinstance(r, dict) and r.to_frame().shape[0] == 7
    assert r.rows()[0]["side"] == "post"
    frame = r.to_frame()
    frame.iloc[0, 0] = 999
    assert r.rows()[0]["row"] == 0
    infused = pf.run_request(
        end=12,
        times=[0.1, 1, 2, 12],
        administrations=[pf.infusion("depot", 300, "mg", duration=1)],
        solver="analytic_linear_pk",
    )
    assert all(np.isfinite(m.run(infused)["outputs"][0]["values"]))
    with pytest.raises(pf.QSPQuestError):
        pf.run_request(end=1, times=[1, 0])
    with pytest.raises(pf.QSPQuestError):
        pf.quantity(float("nan"), "mg")
    msg = pf.scientific_status(
        {"status": "iteration_limit", "boundary_parameters": ["cl"]}
    )
    assert "exploratory" in msg["message"].lower() and msg["warnings"]
    assert (
        "resume"
        in pf.QSPQuestError({"code": "cancelled"}).explain()["message"].lower()
    )
    assert check_environment(analysis=True)["status"] == "passed"


def test_population_resume_preserves_saem_state_and_rng(tmp_path):
    m, subjects, p = cohort()
    for i, s in enumerate(subjects):
        s["covariates"] = {"weight_fraction": 0.7 + i * 0.1}
    options = dict(
        covariance="diagonal",
        max_iterations=80,
        burn_in=20,
        sweeps=1,
        seed=63,
        max_simulations=5000,
        covariates=[
            {
                "parameter": "cl",
                "covariate": "weight_fraction",
                "reference": 1.0,
                "initial": 0.0,
                "lower": -1.0,
                "upper": 1.0,
            }
        ],
    )
    full = m.fit_population(
        subjects,
        p,
        [[0.09]],
        {"additive": {"initial": 0.2, "lower": 0.01, "upper": 1.0}},
        **options
    )
    path = tmp_path / "population.json"
    events = []

    def progress(e):
        events.append(e)
        if e["completed"] == 30:
            control.cancel()

    control = pf.AnalysisControl(
        progress=progress, checkpoint=path, checkpoint_every=10
    )
    with pytest.raises(pf.QSPQuestError) as exception:
        m.fit_population(
            subjects,
            p,
            [[0.09]],
            {"additive": {"initial": 0.2, "lower": 0.01, "upper": 1.0}},
            control=control,
            **options
        )
    assert exception.value.code == "cancelled" and events[-1]["completed"] == 30
    resumed = m.fit_population(
        subjects,
        p,
        [[0.09]],
        {"additive": {"initial": 0.2, "lower": 0.01, "upper": 1.0}},
        control=pf.AnalysisControl(checkpoint=path, resume=True),
        **options
    )
    for key in [
        "parameters",
        "omega",
        "error",
        "trace",
        "eta",
        "fitted_observations",
        "diagnostics",
        "status",
    ]:
        assert resumed[key] == full[key], key
    # Revalidation calls are counted rather than erased from the work budget.
    assert resumed["simulations"] >= full["simulations"]
    wrong = deepcopy(p)
    wrong["cl"]["initial"] = 3.1
    with pytest.raises(pf.QSPQuestError) as exception:
        m.fit_population(
            subjects,
            wrong,
            [[0.09]],
            {"additive": {"initial": 0.2, "lower": 0.01, "upper": 1.0}},
            control=pf.AnalysisControl(checkpoint=path, resume=True),
            **options
        )
    assert exception.value.code == "checkpoint"


def test_population_rejects_checkpoint_with_previous_stopping_rule(tmp_path, monkeypatch):
    import hashlib
    import qspquest.population as population_module

    m, subjects, parameters = cohort()
    path = tmp_path / "previous-rule.json"
    identity = population_module.identity

    def previous_context(model, request):
        request = dict(request)
        request.pop("convergence_window")
        request.pop("initial_chain_equilibration")
        return identity(model, request)

    options = dict(max_iterations=80, burn_in=20, sweeps=1, seed=63)

    def progress(event):
        if event["completed"] == 10:
            control.cancel()

    control = pf.AnalysisControl(checkpoint=path, checkpoint_every=10, progress=progress)
    # Create an integrity-valid checkpoint with the previous context contract.
    with monkeypatch.context() as previous:
        previous.setattr(population_module, "identity", previous_context)
        with pytest.raises(pf.QSPQuestError) as stopped:
            m.fit_population(
                subjects, parameters, [[0.09]], {"additive": 0.1},
                control=control, **options,
            )
        assert stopped.value.code == "cancelled"
    before = hashlib.sha256(path.read_bytes()).hexdigest()
    with pytest.raises(pf.QSPQuestError) as refused:
        m.fit_population(
            subjects, parameters, [[0.09]], {"additive": 0.1},
            control=pf.AnalysisControl(checkpoint=path, resume=True), **options,
        )
    assert refused.value.code == "checkpoint"
    assert "start a new analysis" in str(refused.value)
    assert hashlib.sha256(path.read_bytes()).hexdigest() == before


def test_vpc_resume_and_corruption_rejection(tmp_path):
    m, subjects, p = cohort()
    parameters = {"cl": pf.quantity(3, "L/h")}
    for i, s in enumerate(subjects):
        s["covariates"] = {"weight_fraction": 0.7 + i * 0.1}
    options = dict(
        replicates=8,
        bins=[0, 1, 4, 12],
        seed=97,
        covariates=[
            {
                "parameter": "cl",
                "covariate": "weight_fraction",
                "reference": 1.0,
                "estimate": 0.1,
            }
        ],
    )
    full = m.visual_predictive_check(
        subjects, parameters, [[0.09]], {"additive": 0.2}, **options
    )
    path = tmp_path / "vpc.json"

    def progress(e):
        if e["completed"] == 3:
            control.cancel()

    control = pf.AnalysisControl(checkpoint=path, checkpoint_every=1, progress=progress)
    with pytest.raises(pf.QSPQuestError):
        m.visual_predictive_check(
            subjects,
            parameters,
            [[0.09]],
            {"additive": 0.2},
            control=control,
            **options
        )
    resumed = m.visual_predictive_check(
        subjects,
        parameters,
        [[0.09]],
        {"additive": 0.2},
        control=pf.AnalysisControl(checkpoint=path, resume=True),
        **options
    )
    assert resumed["bins"] == full["bins"]
    payload = json.loads(path.read_text())
    payload["payload"]["state"]["draws"][0][0] += 1
    path.write_text(json.dumps(payload))
    with pytest.raises(pf.QSPQuestError) as exception:
        m.visual_predictive_check(
            subjects,
            parameters,
            [[0.09]],
            {"additive": 0.2},
            control=pf.AnalysisControl(checkpoint=path, resume=True),
            **options
        )
    assert exception.value.code == "checkpoint"


def test_bootstrap_resume_replays_whole_subject_draws(tmp_path):
    m, subjects, p = cohort()
    options = dict(
        replicates=3,
        seed=22,
        max_iterations=40,
        burn_in=10,
        sweeps=1,
        max_simulations=8000,
        covariance="diagonal",
    )
    # Fitted initial bindings override invalid source-run parameter values.
    for subject in subjects:
        subject["run"]["parameters"] = {"cl": pf.quantity(0, "L/h")}
    full = m.bootstrap_population(subjects, p, [[0.09]], {"additive": 0.2}, **options)
    path = tmp_path / "bootstrap.json"

    def progress(e):
        if e["completed"] == 1:
            control.cancel()

    control = pf.AnalysisControl(checkpoint=path, checkpoint_every=1, progress=progress)
    with pytest.raises(pf.QSPQuestError):
        m.bootstrap_population(
            subjects, p, [[0.09]], {"additive": 0.2}, control=control, **options
        )
    resumed = m.bootstrap_population(
        subjects,
        p,
        [[0.09]],
        {"additive": 0.2},
        control=pf.AnalysisControl(checkpoint=path, resume=True),
        **options
    )
    assert resumed["replicates"] == full["replicates"]
    assert resumed["status"] == full["status"]


def test_dataset_bridge_preserves_provenance_and_explicit_errors():
    m, subjects, p = cohort()
    template = {
        "schema": "qspquest.fit/v0.1",
        "problem": {
            "kind": "saem",
            "request": {
                "subjects": [
                    {
                        "simulation": {
                            "schema": "qspquest.sensitivity/v0.1",
                            "with_respect_to": ["cl"],
                            "run": pf.run_request(
                                end=12,
                                times=[1],
                                administrations=[],
                                solver="analytic_linear_pk",
                            ),
                        },
                        "observations": [],
                        "priors": [],
                    }
                ]
            },
        },
    }
    template["problem"]["request"]["subjects"][0]["simulation"]["run"]["regimen"][
        "observations"
    ] = []
    data = [
        {"ID": "A", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 300},
        {"ID": "A", "TIME": 1, "EVID": 0, "CMT": 2, "DV": 6.0},
    ]
    dataset = pf.population_fit_dataset(
        data,
        template,
        compartments={1: "depot"},
        endpoints={
            2: {
                "output": "cp",
                "value_unit": "mg/L",
                "error": {
                    "additive_sd": pf.quantity(0.2, "mg/L"),
                    "proportional_sd": 0.0,
                },
            }
        },
        time_unit="h",
        amount_unit="mg",
    )
    mapped = dataset.to_subjects(covariates={"A": {"weight": 70}})
    assert mapped[0]["observations"][0]["source_row"] == 1
    assert mapped[0]["covariates"]["weight"] == 70
    mapped[0]["run"]["regimen"]["administrations"][0]["amount"]["value"] = 1
    assert (
        dataset.request["problem"]["request"]["subjects"][0]["simulation"]["run"][
            "regimen"
        ]["administrations"][0]["amount"]["value"]
        == 300
    )


def test_checkpoint_read_errors_are_explained(tmp_path):
    for content in [None, "broken JSON", "[]"]:
        path = tmp_path / "invalid.json"
        if content is not None:
            path.write_text(content)
        control = pf.AnalysisControl(checkpoint=path, resume=True)
        with pytest.raises(pf.QSPQuestError) as exception:
            control.start("vpc", {})
        assert exception.value.code == "checkpoint"
        assert "checkpoint" in exception.value.explain()["message"].lower()
