"""Seeded synthetic oral-PK observations with an independent analytic generator."""

import math
import random

from qspquest import CompiledModel, bolus, run_request


def synthetic_subjects(model_source, seed=2026):
    """Generate 12 subjects independently of the numerical simulator."""
    rng = random.Random(seed)
    times = [0.25, 0.5, 1, 2, 4, 8, 12, 24]
    subjects, truth = [], []
    for index in range(12):
        cl = 1.8 * math.exp(rng.gauss(0, 0.15))
        vc = 30 * math.exp(rng.gauss(0, 0.15))
        ka = 1.2 * math.exp(rng.gauss(0, 0.15))
        amount = 180.0
        rate = cl / vc
        values = []
        for time in times:
            concentration = (
                amount
                / vc
                * ka
                / (ka - rate)
                * (math.exp(-rate * time) - math.exp(-ka * time))
            )
            sd = math.sqrt(0.2**2 + (0.1 * concentration) ** 2)
            values.append(concentration + rng.gauss(0, sd))
        run = run_request(
            end=24,
            times=times,
            administrations=[bolus("depot", amount, "mg")],
            solver="analytic_linear_pk",
        )
        observations = [
            {
                "row": i,
                "source_row": index * len(times) + i,
                "output": "cp",
                "value": {"value": value, "unit": "mg/L"},
            }
            for i, value in enumerate(values)
        ]
        subjects.append(
            {"id": str(index + 1), "run": run, "observations": observations}
        )
        truth.append({"subject": str(index + 1), "cl": cl, "vc": vc, "ka": ka})
    return CompiledModel(model_source), subjects, truth


def parameter_specs():
    """Positive bounds for the teaching model."""
    return {
        "cl": {"initial": 2.0, "lower": 0.1, "upper": 10.0, "unit": "L/h"},
        "vc": {"initial": 30.0, "lower": 5.0, "upper": 100.0, "unit": "L"},
        "ka": {"initial": 1.0, "lower": 0.1, "upper": 6.0, "unit": "1/h"},
    }
