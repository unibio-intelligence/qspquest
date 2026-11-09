"""Example figures for the publication-figures guide.

Fits the 12 synthetic oral-PK subjects from oral_pk_analysis.py (seed 2026) and
writes each guide figure with qspquest.plot.save. Requires the analysis and
plot extras. Run from the checkout root:

    python docs/examples/publication_figures.py [OUTPUT_DIRECTORY]

The default output directory is docs/tutorials/figures/.
"""

import copy
from pathlib import Path
import sys

import qspquest as qq

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from oral_pk_analysis import parameter_specs, synthetic_subjects  # noqa: E402

ROOT = HERE.parents[1]
ERROR = {"model": "normal", "additive": 0.2, "proportional": 0.1}
OMEGA = [[0.04, 0, 0], [0, 0.04, 0], [0, 0, 0.04]]


def analyses():
    """Run every analysis shown in the guide; return the native results."""
    model, subjects, _ = synthetic_subjects((ROOT / "examples/synthetic/absorption.qsp").read_text())
    parameters = parameter_specs()
    population = model.fit_population(subjects, parameters, OMEGA, ERROR, seed=11,
                                      covariance="diagonal", sweeps=5, tolerance=0.03,
                                      max_iterations=1000, burn_in=200, max_simulations=1000000)
    assert population["status"] == "converged", population["status"]
    estimates = population["parameters"]
    vpc = model.visual_predictive_check(subjects, estimates, population["omega"], population["error"],
                                        replicates=200, seed=7, prediction_corrected=True,
                                        bins=[0, 0.75, 1.5, 3, 6, 10, 18, 24])
    profile = model.profile_likelihood(subjects[0]["run"], subjects[0]["observations"], parameters,
                                       ERROR, "cl", [1.4, 1.6, 1.8, 2.0, 2.2, 2.4, 2.7, 3.0],
                                       max_simulations=200000)
    once = subjects[0]["run"]
    twice = copy.deepcopy(once)
    dose = twice["regimen"]["administrations"][0]
    dose["amount"]["value"] = 90.0
    second = copy.deepcopy(dose)
    second["time"]["value"] = 12.0
    twice["regimen"]["administrations"].append(second)
    regimens = {"180 mg once": once, "90 mg twice": twice}
    comparison = model.compare_regimens(regimens, estimates, population["omega"], "cp",
                                        subjects=200, seed=5)
    # Same virtual subjects with clearance reduced by 30%, for the model overlay figures.
    reduced = copy.deepcopy(estimates)
    reduced["cl"]["value"] *= 0.7
    reduced_comparison = model.compare_regimens(regimens, reduced, population["omega"], "cp",
                                                subjects=200, seed=5)
    run = qq.run_request(end=24, times=[1, 4, 24], administrations=[qq.bolus("depot", 180, "mg")],
                         solver="analytic_linear_pk")
    ranges = [("cl", 1.0, 3.0, "L/h"), ("vc", 20.0, 45.0, "L"), ("ka", 0.5, 2.0, "1/h")]
    morris = model.morris({"schema": "qspquest.morris/v0.1", "problem": {"kind": "generated", "request": {
        "run": run, "output": "cp", "row": 1, "trajectory_count": 6, "num_levels": 4, "seed": 3,
        "ranges": [{"parameter": name, "lower": qq.quantity(low, unit),
                    "upper": qq.quantity(high, unit)} for name, low, high, unit in ranges],
        "total_budgets": {"solver_callbacks": 4000000, "events": 4000, "output_values": 400000}}}})
    return {"population": population, "vpc": vpc, "profile": profile,
            "comparison": comparison, "morris": morris,
            "models": {"Estimated": comparison, "Clearance −30%": reduced_comparison}}


def figures(results):
    """Build the guide figures from the results, in guide order."""
    population, comparison = results["population"], results["comparison"]
    return {
        "publication-regimens": qq.plot.regimens(comparison, label="Plasma concentration"),
        "publication-individual-fits": qq.plot.profile(population, label="Plasma concentration",
                                                       subjects=["1", "2", "3", "4"]),
        "publication-goodness-of-fit": qq.plot.goodness_of_fit(population),
        "publication-pcvpc": qq.plot.vpc(results["vpc"], label="concentration"),
        "publication-likelihood-profile": qq.plot.likelihood_profile(results["profile"]),
        "publication-exposure-ratio": qq.plot.exposure_ratio(comparison),
        "publication-morris": qq.plot.morris(results["morris"], label="cp at 4 h"),
        "publication-model-bands": qq.plot.regimens(results["models"], label="Plasma concentration",
                                                    target=2.0),
        "publication-target-attainment": qq.plot.target_attainment(
            results["models"], "endpoint", 2.0, direction="above"),
    }


def main():
    output = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs/tutorials/figures"
    results = analyses()
    population = results["population"]
    print("population fit:", population["status"], population["iterations"], "iterations;",
          {name: round(q["value"], 3) for name, q in population["parameters"].items()})
    print("pcVPC:", results["vpc"]["status"], results["vpc"]["successful_replicates"], "replicates")
    print("profile interval for cl:", results["profile"]["interval"], results["profile"]["unit"])
    print(qq.tables.target_attainment(results["models"], "endpoint", 2.0, direction="above")
          [["model", "regimen", "met", "subjects", "attainment", "lower", "upper"]].to_string(index=False))
    for name, figure in figures(results).items():
        for path in qq.plot.save(figure, output / f"{name}.png", dpi=200):
            print(path.relative_to(ROOT) if path.is_relative_to(ROOT) else path)


if __name__ == "__main__":
    main()
