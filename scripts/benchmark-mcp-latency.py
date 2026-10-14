#!/usr/bin/env python3
"""Measure local/hosted MCP execution and explicit async routing separately.

Requires jsonschema. Retains every observation, including errors/timeouts; does
not claim completed scientific work for an async_required admission response.
"""
import argparse
import json
import math
from pathlib import Path
import time
import urllib.error
import urllib.request
import zipfile


def nearest_rank(values, percentile=0.95):
    if not values:
        return None
    if not 0 < percentile <= 1:
        raise ValueError("percentile must be in (0,1]")
    return sorted(values)[math.ceil(percentile * len(values)) - 1]


def summarize(rows):
    warm = [r for r in rows if r["repeat"] == 1]
    interactive = [r for r in warm if not r["heavy_workload"]]
    admission = [r for r in warm if r["heavy_workload"]]
    return {
        "percentile_method": "nearest rank: sorted[ceil(p*n)-1]",
        "warm_all_call_p95_ms": nearest_rank([r["elapsed_ms"] for r in warm]),
        "interactive_execution_p95_ms": nearest_rank(
            [r["elapsed_ms"] for r in interactive]
        ),
        "heavy_route_response_p95_ms": nearest_rank(
            [r["elapsed_ms"] for r in admission]
        ),
        "scientific_successes": sum(r["scientific_success"] for r in rows),
        "async_required_responses": sum(r["async_required"] for r in rows),
        "failures": sum(
            not r["scientific_success"] and not r["async_required"] for r in rows
        ),
        "interactive_target_met": bool(interactive)
        and all(r["scientific_success"] for r in interactive)
        and nearest_rank([r["elapsed_ms"] for r in interactive]) <= 500,
        "heavy_route_target_met": bool(admission)
        and all(r["async_required"] for r in admission)
        and nearest_rank([r["elapsed_ms"] for r in admission]) <= 500,
        "governed_job_admission_measured": False,
    }


def main():
    from jsonschema import validators

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("endpoint")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    library = root / "bindings/mcp/assets/library"
    schema = json.loads((root / "spec/schema/run.schema.json").read_text())
    validator = validators.validator_for(schema)(schema)
    models = []
    for model in json.loads((library / "catalog.json").read_text())["models"]:
        with zipfile.ZipFile(
            library / model.get("archive", model.get("archive_path"))
        ) as archive:
            document = json.loads(archive.read("native/model.json"))
            for scenario in model["scenario_ids"]:
                validator.validate(
                    json.loads(archive.read("native/scenarios/" + scenario + ".json"))
                )
                models.append(
                    (
                        model["id"],
                        scenario,
                        document["outputs"][0]["name"],
                        len(document["states"]),
                    )
                )
    rows = []
    for repeat in range(2):
        for model, scenario, output, states in models:
            body = {
                "jsonrpc": "2.0",
                "id": len(rows) + 1,
                "method": "tools/call",
                "params": {
                    "name": "simulate",
                    "arguments": {
                        "model": {
                            "scenario": {"model_id": model, "scenario_id": scenario}
                        },
                        "outputs": [output],
                    },
                },
            }
            request = urllib.request.Request(
                args.endpoint.rstrip("/") + "/mcp",
                data=json.dumps(body).encode(),
                headers={
                    "Content-Type": "application/json",
                    "Accept": "application/json, text/event-stream",
                    "MCP-Protocol-Version": "2025-11-25",
                },
            )
            start = time.perf_counter()
            try:
                with urllib.request.urlopen(request, timeout=55) as response:
                    result = json.load(response)["result"]
                error = result.get("structuredContent", {}).get("error")
                success = not result.get("isError", False)
            except (urllib.error.URLError, TimeoutError, ValueError, KeyError) as exc:
                error, success = {"code": "transport", "message": str(exc)}, False
            row = {
                "repeat": repeat,
                "model": model,
                "scenario": scenario,
                "states": states,
                "heavy_workload": states >= 64,
                "elapsed_ms": (time.perf_counter() - start) * 1000,
                "scientific_success": success,
                "async_required": bool(error and error.get("code") == "async_required"),
                "error": error,
            }
            rows.append(row)
            print(
                repeat,
                model,
                scenario,
                round(row["elapsed_ms"], 2),
                error or "completed",
                flush=True,
            )
    args.output.write_text(
        json.dumps(
            {
                "scope": "HTTP MCP, one selected readout; full state and dynamics checks",
                "schema_validated_scenarios": len(models),
                "targets_ms": {
                    "interactive_execution_p95": 500,
                    "governed_job_admission_p95": 500,
                },
                "rows": rows,
                **summarize(rows),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
