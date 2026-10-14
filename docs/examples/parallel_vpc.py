"""Synthetic, repeatable VPC with bounded workers and complete failure receipts."""
import argparse
from copy import deepcopy
import json
import math
from pathlib import Path

import qspquest as qsp


def synthetic_cohort():
    root = Path(__file__).resolve().parents[2]
    model = qsp.CompiledModel((root / "conformance/models/synthetic-one-compartment.qsp").read_text())
    times = [0.5, 2, 8, 24]
    run = qsp.run_request(end=24, times=times,
                          administrations=[qsp.bolus("central", 6, "mg")],
                          solver="diffsol_bdf")
    subjects = []
    for i, clearance in enumerate([0.22, 0.26, 0.29, 0.32, 0.36, 0.42]):
        observations = [{"row": row, "output": "cp", "value": qsp.quantity(
            2 * math.exp(-clearance / 3 * time) + 0.005 * math.sin(i + row), "mg/L")}
            for row, time in enumerate(times)]
        subjects.append({"id": f"synthetic-{i}", "run": deepcopy(run), "observations": observations})
    return model, subjects


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    model, subjects = synthetic_cohort()
    result = model.visual_predictive_check(subjects, {"cl": qsp.quantity(0.3, "L/h")},
        [[0.09]], {"additive": 0.02}, bins=[0, 2, 8, 24], replicates=16,
        random_effects=["cl"], seed=1729, max_simulations=200, workers=args.workers)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, indent=2, allow_nan=False)
        output.write("\n")
    print(json.dumps({"status": result["status"], "successful_replicates": result["successful_replicates"],
        "failed_replicates": result["failed_replicates"], "simulations": result["simulations"],
        "output": str(args.output)}))


if __name__ == "__main__":
    main()
