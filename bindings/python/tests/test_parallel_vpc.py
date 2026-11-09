"""Independent-work determinism, failure denominators and bounded cancellation."""
from copy import deepcopy
import json

import pytest
import qspquest as qq
from test_workflow import cohort


def settings():
    return dict(replicates=8, bins=[0, 1, 4, 12], seed=97)


def predictive(model, subjects, workers, error=None, **options):
    return qq.visual_predictive_check(model, subjects, {"cl": qq.quantity(3, "L/h")},
                                      [[0.09]], error or {"additive": 0.2}, workers=workers,
                                      **settings(), **options)


def test_worker_count_preserves_random_streams_rows_and_scientific_identity():
    model, subjects, _ = cohort()
    serial = predictive(model, subjects, 1)
    for workers in [2, 4, 8]:
        result = predictive(model, subjects, workers)
        for key in ["bins", "replicate_records", "identity", "seed", "simulations"]:
            assert result[key] == serial[key], key
        assert result["status"] == "complete"
        assert result["successful_replicates"] == 8
        assert result["execution"]["random_stream"] == "splitmix64_box_muller_substream/v1"
        assert len({r["stream_id"] for r in result["replicate_records"]}) == 8


def test_domain_failures_are_retained_without_confidence_intervals():
    model, subjects, _ = cohort()
    # A pre-absorption sample has zero concentration, which lognormal
    # observation error cannot draw; every replicate fails on that subject.
    first = deepcopy(subjects[0])
    first["run"]["regimen"]["observations"].insert(0, {"time": qq.quantity(0, "h"), "side": "pre"})
    for o in first["observations"]:
        o["row"] += 1
    first["observations"].insert(0, {"row": 0, "output": "cp", "value": qq.quantity(0.01, "mg/L")})
    subjects = [first] + subjects[1:]
    error = {"model": "lognormal", "proportional": 0.2}
    serial = predictive(model, subjects, 1, error)
    parallel = predictive(model, subjects, 3, error)
    assert serial["bins"] == parallel["bins"]
    assert serial["replicate_records"] == parallel["replicate_records"]
    assert serial["failed_replicates"] == 8
    assert serial["successful_replicates"] == 0
    assert serial["status"] == "incomplete"
    assert [r["replicate"] for r in serial["replicate_records"]] == list(range(8))
    assert all(r["code"] == "domain" for r in serial["replicate_records"])
    assert all(cell["blq_confidence_interval"] is None for cell in serial["bins"])
    assert all(q["confidence_interval"] is None for cell in serial["bins"] for q in cell["quantiles"])
    assert serial["simulations"] == len(subjects) + 8
    json.dumps(parallel, allow_nan=False)
    explanation = qq.scientific_status(parallel)
    assert explanation['status'] == 'incomplete'
    assert any('Confidence intervals are suppressed' in warning for warning in explanation['warnings'])


def test_parallel_cancel_resume_can_change_workers_and_retains_dispatched_cost(tmp_path):
    model, subjects, _ = cohort()
    reference = predictive(model, subjects, 1)
    path = tmp_path / "parallel-vpc.json"

    def progress(event):
        if event["completed"] == 3:
            control.cancel()

    control = qq.AnalysisControl(checkpoint=path, checkpoint_every=1, progress=progress)
    with pytest.raises(qq.QSPQuestError) as error:
        predictive(model, subjects, 3, control=control)
    assert error.value.code == "cancelled"
    saved = json.loads(path.read_text())["payload"]["state"]
    assert len(saved["draws"]) == len(saved["replicate_records"]) == 3
    # The checkpoint includes calls charged by workers which drained on cancel.
    assert saved["simulations"] >= len(subjects) * 4
    resumed = predictive(model, subjects, 2,
                         control=qq.AnalysisControl(checkpoint=path, resume=True))
    assert resumed["bins"] == reference["bins"]
    assert resumed["replicate_records"] == reference["replicate_records"]
    assert resumed["simulations"] >= reference["simulations"] + len(subjects)


def test_invalid_workers_and_unaffordable_design_fail_before_native_calls():
    model, subjects, _ = cohort()
    # Any simulation of these subjects fails with work_budget, so an admission
    # error shows that no simulation ran before the request was refused.
    unrunnable = deepcopy(subjects)
    for subject in unrunnable:
        subject["run"]["budgets"]["solver_callbacks"] = 1
    with pytest.raises(qq.QSPQuestError) as error:
        predictive(model, unrunnable, 2)
    assert error.value.code == "work_budget"
    for workers in [0, 9, True, 1.5]:
        with pytest.raises(qq.QSPQuestError) as error:
            predictive(model, unrunnable, workers)
        assert error.value.code == "invalid_input"
    with pytest.raises(qq.QSPQuestError) as error:
        predictive(model, unrunnable, 2, max_simulations=53)
    assert error.value.code == "budget"


@pytest.mark.parametrize("mutation", ["missing_state", "bad_simulations", "nonboolean_censor", "bad_stream"])
def test_malformed_restart_state_has_actionable_checkpoint_failure(tmp_path, mutation):
    from qspquest.control import _digest
    model, subjects, _ = cohort()
    path = tmp_path / "invalid-state.json"
    control = qq.AnalysisControl(checkpoint=path, checkpoint_every=1)

    def progress(event):
        if event["completed"] == 1:
            control.cancel()

    control.progress = progress
    with pytest.raises(qq.QSPQuestError):
        predictive(model, subjects, 2, control=control)
    document = json.loads(path.read_text())
    state = document["payload"]["state"]
    if mutation == "missing_state":
        state.pop("draws")
    elif mutation == "bad_simulations":
        state["simulations"] = "many"
    elif mutation == "nonboolean_censor":
        state["censored"][0][0] = 1
    else:
        state["replicate_records"][0]["stream_id"] = "invalid"
    document["sha256"] = _digest(document["payload"])
    path.write_text(json.dumps(document))
    with pytest.raises(qq.QSPQuestError) as error:
        predictive(model, subjects, 2, control=qq.AnalysisControl(checkpoint=path, resume=True))
    assert error.value.code == "checkpoint"


def test_checkpoints_from_the_earlier_implementation_are_rejected(tmp_path):
    from qspquest.control import _digest
    model, subjects, _ = cohort()
    path = tmp_path / "earlier.json"
    payload = {"context": {}, "state": {}}
    path.write_text(json.dumps({"schema": "qspquest.checkpoint/v0.1", "sha256": _digest(payload), "payload": payload}))
    with pytest.raises(qq.QSPQuestError) as error:
        predictive(model, subjects, 1, control=qq.AnalysisControl(checkpoint=path, resume=True))
    assert error.value.code == "checkpoint"
    assert "start a new analysis" in str(error.value)
