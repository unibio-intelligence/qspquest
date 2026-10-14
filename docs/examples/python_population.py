"""Fit synthetic Gaussian log-effects; optionally verify cancellation and resume."""

import argparse
import math
from pathlib import Path
import random

from qspquest import AnalysisControl, CompiledModel, QSPQuestError, quantity, run_request

SOURCE = """model gaussian_population_example
time_unit = h
parameters
  theta = 2.718281828459045 [1] log
  k = 1 [1/h]
states
  anchor = 0 [1]
dynamics
  d/dt(anchor) = -k * anchor
outputs
  response = log(theta) + anchor [1]
end
"""

PARAMETERS = {
    "theta": {"initial": math.e, "lower": 0.5, "upper": 20.0, "unit": "1"}
}
OPTIONS = dict(
    max_iterations=2000,
    burn_in=200,
    sweeps=2,
    tolerance=0.003,
    max_simulations=100000,
    seed=17029,
)


def cohort():
    rng = random.Random(40211)
    subjects, means = [], []
    for index in range(24):
        eta = rng.gauss(0.0, 0.4)
        values = [1.4 + eta + rng.gauss(0.0, 0.25) for _ in range(6)]
        means.append(sum(values) / len(values))
        subjects.append({
            "id": f"subject-{index:02d}",
            "run": run_request(
                end=5.0, times=[0, 1, 2, 3, 4, 5], solver="diffsol_bdf"
            ),
            "observations": [
                {"row": row, "output": "response", "value": quantity(value, "1")}
                for row, value in enumerate(values)
            ],
        })
    # The row mean has variance Omega + residual_variance / rows_per_subject.
    mean = sum(means) / len(means)
    variance = sum((value - mean) ** 2 for value in means) / len(means) - 0.25**2 / 6
    return subjects, mean, variance


def fit(model, subjects, control=None):
    return model.fit_population(
        subjects, PARAMETERS, [[0.04]], {"additive": 0.25},
        control=control, **OPTIONS,
    )


def verify_restart(model, subjects, full, directory):
    checkpoint = directory / "population.json"

    def progress(event):
        if event["completed"] == 100:
            control.cancel()

    control = AnalysisControl(
        checkpoint=checkpoint, checkpoint_every=10, progress=progress
    )
    try:
        fit(model, subjects, control)
    except QSPQuestError as error:
        if error.code != "cancelled":
            raise
    else:
        raise RuntimeError("The cancellation demonstration did not stop")
    resumed = fit(model, subjects, AnalysisControl(checkpoint=checkpoint, resume=True))
    for key in [
        "status", "parameters", "omega", "error", "trace", "eta",
        "objective", "fitted_observations", "diagnostics",
    ]:
        if resumed[key] != full[key]:
            raise RuntimeError(f"Resumed {key} differs from uninterrupted analysis")
    print("Verified identical estimates, trace and RNG continuation after resume.")
    print(f"Revalidation adds {resumed['simulations'] - full['simulations']} simulations.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--restart-demo", type=Path, metavar="NEW_DIRECTORY",
        help="save a synthetic checkpoint, cancel at iteration 100 and resume",
    )
    args = parser.parse_args()
    if args.restart_demo is not None:
        try:
            args.restart_demo.mkdir(parents=True, exist_ok=False)
        except FileExistsError:
            parser.error("--restart-demo requires a new directory")
    model = CompiledModel(SOURCE)
    subjects, mean, variance = cohort()
    result = fit(model, subjects)
    print(result.summary())
    print(f"Gaussian reference: theta={math.exp(mean):.6f}, Omega={variance:.6f}")
    print(f"Estimated log-effect variance Omega={result['omega'][0][0]:.6f}")
    print(f"Native simulations: {result['simulations']}; iterations: {result['iterations']}")
    if result["status"] == "converged":
        log_mean = math.log(result["parameters"]["theta"]["value"])
        if abs(log_mean - mean) > 0.02 or abs(result["omega"][0][0] - variance) > 0.03:
            raise RuntimeError("This synthetic fit does not match its Gaussian reference")
        print("Verified the synthetic Gaussian mean and variance comparison.")
    else:
        print("Convergence is unresolved; interpret these estimates as diagnostic.")
    print("The SAEM objective is a complete-data surrogate; do not use it for likelihood ratios.")
    if args.restart_demo is not None:
        verify_restart(model, subjects, result, args.restart_demo)


if __name__ == "__main__":
    main()
