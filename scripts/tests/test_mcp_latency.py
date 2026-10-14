import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "latency", Path(__file__).parents[1] / "benchmark-mcp-latency.py"
)
latency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(latency)


def test_nearest_rank_keeps_timeouts_and_correct_tail():
    assert latency.nearest_rank(list(range(1, 48))) == 45
    assert latency.nearest_rank([10, 20, 45000]) == 45000
    assert latency.nearest_rank([]) is None


def test_routing_is_not_scientific_or_job_admission_success():
    rows = [
        {
            "repeat": 1,
            "heavy_workload": False,
            "elapsed_ms": 20,
            "scientific_success": True,
            "async_required": False,
        },
        {
            "repeat": 1,
            "heavy_workload": True,
            "elapsed_ms": 5,
            "scientific_success": False,
            "async_required": True,
        },
    ]
    result = latency.summarize(rows)
    assert result["scientific_successes"] == 1
    assert result["async_required_responses"] == 1
    assert result["interactive_target_met"] and result["heavy_route_target_met"]
    assert not result["governed_job_admission_measured"]
