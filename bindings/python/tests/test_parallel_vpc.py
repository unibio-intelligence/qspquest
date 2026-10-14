"""Independent-work determinism, failure denominators and bounded cancellation."""
from concurrent.futures import ThreadPoolExecutor
import json
import threading

import pytest
import qspquest as qsp
from qspquest._analysis import Budget
from qspquest._work import ordered_work
from test_friendly import cohort


def settings():
    return dict(replicates=8, bins=[0, 1, 4, 12], seed=97)


def predictive(model, subjects, workers, **options):
    return qsp.visual_predictive_check(model, subjects, {"cl": qsp.quantity(3, "L/h")},
                                      [[0.09]], {"additive": 0.2}, workers=workers,
                                      **settings(), **options)


def test_worker_count_preserves_random_streams_rows_and_scientific_identity():
    model, subjects, _ = cohort()
    serial = predictive(model, subjects, 1)
    for workers in [2, 4]:
        result = predictive(model, subjects, workers)
        for key in ["bins", "replicate_records", "identity", "seed", "simulations"]:
            assert result[key] == serial[key], key
        assert result["status"] == "complete"
        assert result["successful_replicates"] == 8
        assert len({r["stream_id"] for r in result["replicate_records"]}) == 8


def test_domain_failures_are_retained_without_confidence_intervals():
    model, subjects, _ = cohort()

    class FailedDraws:
        model_content_hash = model.model_content_hash

        def run(self, request):
            if request["parameters"]["cl"]["value"] != 3:
                raise qsp.QSPQuestError({"code": "domain", "message": "Synthetic random-effect rejection"})
            return model.run(request)

    serial = predictive(FailedDraws(), subjects, 1)
    parallel = predictive(FailedDraws(), subjects, 3)
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
    explanation = qsp.scientific_status(parallel)
    assert explanation['status'] == 'incomplete'
    assert any('Confidence intervals are suppressed' in warning for warning in explanation['warnings'])


def test_parallel_cancel_resume_can_change_workers_and_retains_dispatched_cost(tmp_path):
    model, subjects, _ = cohort()
    reference = predictive(model, subjects, 1)
    path = tmp_path / "parallel-vpc.json"

    def progress(event):
        if event["completed"] == 3:
            control.cancel()

    control = qsp.AnalysisControl(checkpoint=path, checkpoint_every=1, progress=progress)
    with pytest.raises(qsp.QSPQuestError) as error:
        predictive(model, subjects, 3, control=control)
    assert error.value.code == "cancelled"
    saved = json.loads(path.read_text())["payload"]["state"]
    assert len(saved["draws"]) == len(saved["replicate_records"]) == 3
    # The checkpoint includes calls charged by workers which drained on cancel.
    assert saved["simulations"] >= len(subjects) * 4
    resumed = predictive(model, subjects, 2,
                         control=qsp.AnalysisControl(checkpoint=path, resume=True))
    assert resumed["bins"] == reference["bins"]
    assert resumed["replicate_records"] == reference["replicate_records"]
    assert resumed["simulations"] >= reference["simulations"] + len(subjects)


def test_in_flight_work_is_bounded_until_the_owner_accepts_a_result():
    began, lock = [], threading.Lock()
    all_started = threading.Event()

    def execute(index):
        with lock:
            began.append(index)
            if len(began) == 3:
                all_started.set()
        assert all_started.wait(5), "Three independent units must overlap"
        return index

    with ordered_work(range(20), execute, 3) as results:
        assert next(results) == 0
        assert sorted(began) == [0, 1, 2]
        assert list(results) == list(range(1, 20))


def test_aggregate_budget_cannot_be_overspent_by_concurrent_callers():
    class Counter:
        def __init__(self):
            self.lock, self.calls = threading.Lock(), 0

        def run(self, request):
            with self.lock:
                self.calls += 1
            return request

    model, budget = Counter(), Budget(9)

    def execute(index):
        try:
            budget.run(model, index)
            return "success"
        except qsp.QSPQuestError as error:
            assert error.code == "budget"
            return "budget"

    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(execute, range(64)))
    assert results.count("success") == budget.used == model.calls == 9


def test_invalid_workers_and_unaffordable_design_fail_before_native_calls():
    class NoCalls:
        def run(self, request):
            raise AssertionError("Invalid admission must not execute")

    _, subjects, _ = cohort()
    for workers in [0, 9, True, 1.5]:
        with pytest.raises(qsp.QSPQuestError) as error:
            predictive(NoCalls(), subjects, workers)
        assert error.value.code == "invalid_input"
    with pytest.raises(qsp.QSPQuestError) as error:
        predictive(NoCalls(), subjects, 2, max_simulations=53)
    assert error.value.code == "budget"


@pytest.mark.parametrize("mutation", ["missing_state", "bad_rng", "nonboolean_censor", "bad_stream"])
def test_malformed_restart_state_has_actionable_checkpoint_failure(tmp_path, mutation):
    from qspquest.control import _digest
    model, subjects, _ = cohort()
    path = tmp_path / "invalid-state.json"
    control = qsp.AnalysisControl(checkpoint=path, checkpoint_every=1)

    def progress(event):
        if event["completed"] == 1:
            control.cancel()

    control.progress = progress
    with pytest.raises(qsp.QSPQuestError):
        predictive(model, subjects, 2, control=control)
    document = json.loads(path.read_text())
    state = document["payload"]["state"]
    if mutation == "missing_state":
        state.pop("rng")
    elif mutation == "bad_rng":
        state["rng"] = {}
    elif mutation == "nonboolean_censor":
        state["censored"][0][0] = 1
    else:
        state["replicate_records"][0]["stream_id"] = "invalid"
    document["sha256"] = _digest(document["payload"])
    path.write_text(json.dumps(document))
    with pytest.raises(qsp.QSPQuestError) as error:
        predictive(model, subjects, 2, control=qsp.AnalysisControl(checkpoint=path, resume=True))
    assert error.value.code == "checkpoint"
