"""Synthetic oral one-compartment population PK analysis with recovery checks.

Simulates a single-dose oral cohort as population PK event records, fits it with
Python SAEM (full model) and with native FOCEI and SAEM (reduced models),
evaluates the full model by goodness-of-fit rows, a prediction-corrected VPC
and a subject bootstrap, and asserts recovery of the simulation values.
"""

import argparse
import csv
import json
import math
import time
from pathlib import Path

import numpy as np

import qspquest as qq

MODEL_SOURCE = """model oral_one_compartment
time_unit = h
parameters
  cl = 2.4 [L/h] log
  v = 60 [L] log
  ka = 0.9 [1/h] log
states
  depot = 0 [mg]
  central = 0 [mg]
dynamics
  d/dt(depot) = input_depot - ka * depot
  d/dt(central) = ka * depot - cl / v * central
outputs
  cp = central / v [mg/L]
dosing
  depot [mg] = 1
end
"""

# Simulation design and generating values.
SEED = 20261106
SUBJECTS = 60
DOSE_MG = 200.0
TIMES_H = [0.5, 1, 2, 3, 4, 6, 8, 12, 24, 48]
REFERENCE_WT_KG = 70.0
TRUE = {
    "cl": 2.4,  # L/h, CL/F at 70 kg
    "v": 60.0,  # L, V/F
    "ka": 0.9,  # 1/h
    "omega": {"cl": 0.09, "v": 0.04, "ka": 0.16},
    "wt_exponent": 0.75,
    "additive_sd": 0.03,  # mg/L
    "proportional_sd": 0.10,
}

# Recovery tolerances for the full-model estimates.
TOLERANCE = {
    "cl": 0.15,  # relative
    "v": 0.15,  # relative
    "ka": 0.25,  # relative
    "omega": 0.60,  # relative, each diagonal element
    "wt_exponent": 0.40,  # absolute
    "additive_sd": 0.02,  # absolute, mg/L
    "proportional_sd": 0.25,  # relative
    "native_cl": 0.15,  # relative
}

UNITS = {"cl": "L/h", "v": "L", "ka": "1/h"}
DERIVATIVE_TOLERANCE_UNITS = {"cl": "mg.h/L", "v": "mg/L", "ka": "mg.h"}
VPC_BINS_H = [0, 0.75, 1.5, 2.5, 3.5, 5, 7, 10, 18, 36, 48]


def q(value, unit):
    return {"value": value, "unit": unit}


def simulate_cohort(seed=SEED):
    """Return event records, body weights and subject-level simulated values."""
    rng = np.random.default_rng(seed)
    rows, weights, subjects = [], {}, []
    for index in range(1, SUBJECTS + 1):
        wt = float(np.clip(REFERENCE_WT_KG * math.exp(rng.normal(0.05, 0.25)), 40, 140))
        eta = {
            name: float(rng.normal(0, math.sqrt(TRUE["omega"][name])))
            for name in ("cl", "v", "ka")
        }
        cl = TRUE["cl"] * (wt / REFERENCE_WT_KG) ** TRUE["wt_exponent"] * math.exp(eta["cl"])
        v = TRUE["v"] * math.exp(eta["v"])
        ka = TRUE["ka"] * math.exp(eta["ka"])
        k = cl / v
        weights[str(index)] = wt
        subjects.append({"ID": index, "WT": wt, "CL": cl, "V": v, "KA": ka,
                         **{f"ETA_{n.upper()}": e for n, e in eta.items()}})
        rows.append({"ID": index, "TIME": 0.0, "DV": 0.0, "AMT": DOSE_MG,
                     "EVID": 1, "CMT": 1, "MDV": 1})
        for t in TIMES_H:
            # Analytical oral one-compartment concentration.
            c = DOSE_MG / v * ka / (ka - k) * (math.exp(-k * t) - math.exp(-ka * t))
            sd = math.sqrt(TRUE["additive_sd"] ** 2 + (TRUE["proportional_sd"] * c) ** 2)
            rows.append({"ID": index, "TIME": float(t), "DV": c + float(rng.normal(0, sd)),
                         "AMT": 0.0, "EVID": 0, "CMT": 2, "MDV": 0})
    return rows, weights, subjects


def template(kind, selected, parameters):
    """Population request template with one empty subject objective."""
    run = qq.run_request(end=48, times=[48.0], solver="diffsol_bdf")
    run["regimen"]["observations"] = []
    run["parameters"] = {name: q(value, UNITS[name]) for name, value in parameters.items()}
    tolerances = {
        name: {state: q(1e-9, DERIVATIVE_TOLERANCE_UNITS[name])
               for state in ("depot", "central")}
        for name in selected
    }
    return {"schema": "qspquest.fit/v0.1", "problem": {"kind": kind, "request": {
        "subjects": [{"simulation": {"schema": "qspquest.sensitivity/v0.1", "run": run,
                                     "with_respect_to": selected,
                                     "derivative_absolute_tolerances": tolerances},
                      "observations": [], "priors": []}],
        "fixed_effects": [], "random_effects": [], "omega": [],
        "omega_diagonal_bounds": [], "eta_bound": 3.0,
        "max_evaluations": 100_000, "max_iterations": 400,
        "gradient_tolerance": 0.005, "seed": 17,
    }}}


def prepare(rows, request, additive_sd, proportional_sd):
    """Convert the records with depot doses (CMT 1) and cp observations (CMT 2)."""
    endpoints = {2: {"output": "cp", "value_unit": "mg/L", "error": {
        "additive_sd": q(additive_sd, "mg/L"), "proportional_sd": proportional_sd}}}
    return qq.population_fit_dataset(
        rows, request, compartments={1: "depot"}, endpoints=endpoints,
        time_unit="h", amount_unit="mg", observation_side="post")


def full_model_inputs():
    """Starting values, bounds, Omega, residual error and covariate for SAEM."""
    parameters = {
        "cl": {"initial": 2.0, "lower": 0.2, "upper": 20.0, "unit": "L/h"},
        "v": {"initial": 45.0, "lower": 5.0, "upper": 500.0, "unit": "L"},
        "ka": {"initial": 1.5, "lower": 0.05, "upper": 10.0, "unit": "1/h"},
    }
    omega = (np.eye(3) * 0.2).tolist()
    error = {"additive": {"initial": 0.1, "lower": 0.001, "upper": 1.0},
             "proportional": {"initial": 0.2, "lower": 0.01, "upper": 1.0}}
    covariates = [{"parameter": "cl", "covariate": "LOGWT70", "reference": 0.0,
                   "initial": 0.0, "lower": -3.0, "upper": 3.0}]
    return parameters, omega, error, covariates


SAEM_OPTIONS = {"covariance": "diagonal", "max_iterations": 1000, "burn_in": 200,
                "sweeps": 2, "tolerance": 0.03}


def fit_full_model(model, dataset, weights):
    parameters, omega, error, covariates = full_model_inputs()
    log_weight = {sid: {"LOGWT70": math.log(weights[sid] / REFERENCE_WT_KG)}
                  for sid in dataset.subject_ids}
    fit = dataset.fit(model, parameters, omega, error, covariates_by_subject=log_weight,
                      covariates=covariates, seed=SEED % 2**32,
                      max_simulations=1_000_000, **SAEM_OPTIONS)
    return fit, dataset.to_subjects(covariates=log_weight)


def native_fit(rows, kind, fixed, error, weights=None, saem_estimate=None):
    """One-effect native fit of CL/F with V/F, KA and residual error fixed."""
    request = template(kind, ["cl"], {"cl": 2.0, **fixed})
    body = request["problem"]["request"]
    body["fixed_effects"] = [{"parameter": "cl", "lower": q(0.2, "L/h"),
                              "upper": q(20.0, "L/h")}]
    body["random_effects"] = [{"parameter": "cl"}]
    body["omega"] = [[0.2]]
    body["omega_diagonal_bounds"] = [[0.001, 2.0]]
    dataset = prepare(rows, request, error["additive"], error["proportional"])
    if weights is not None:
        body = dataset.request["problem"]["request"]
        body["covariate_effect"] = {
            "parameter": "cl", "covariate": "LOGWT70",
            "subject_values": [math.log(weights[sid] / REFERENCE_WT_KG)
                               for sid in dataset.subject_ids],
            "reference": 0.0, "initial": 0.0, "bounds": [-3.0, 3.0]}
    model = qq.CompiledSensitivities(MODEL_SOURCE, ["cl"])
    return model.fit_population_data(dataset)["result"]["population"]


def gof_rows(fit, weights):
    """PRED, IPRED and conditional standardized residuals by source row."""
    diagnostics = qq.fit_diagnostics(fit)
    rows = []
    for r in diagnostics["observations"]:
        rows.append({"ID": r["subject_id"], "TIME": r["time"], "WT": weights[r["subject_id"]],
                     "DV": r["observed"], "PRED": r["prediction"],
                     "IPRED": r["individual_prediction"],
                     "CRES": r["standardized_residual"], "SOURCE_ROW": r["source_row"]})
    return rows, diagnostics


def estimates(fit):
    omega = np.asarray(fit["omega"])
    return {
        "cl": fit["parameters"]["cl"]["value"],
        "v": fit["parameters"]["v"]["value"],
        "ka": fit["parameters"]["ka"]["value"],
        "omega": {name: float(omega[i, i]) for i, name in enumerate(fit["random_effects"])},
        "wt_exponent": fit["covariate_effects"][0]["estimate"],
        "additive_sd": fit["error"]["additive"],
        "proportional_sd": fit["error"]["proportional"],
    }


def run_vpc(model, subjects, fit, est, replicates, workers):
    return model.visual_predictive_check(
        subjects, {name: q(est[name], UNITS[name]) for name in ("cl", "v", "ka")},
        fit["omega"], {"additive": est["additive_sd"], "proportional": est["proportional_sd"]},
        random_effects=fit["random_effects"],
        covariates=[{"parameter": "cl", "covariate": "LOGWT70", "reference": 0.0,
                     "estimate": est["wt_exponent"]}],
        bins=VPC_BINS_H, replicates=replicates, prediction_corrected=True,
        seed=4099, workers=workers, max_simulations=replicates * SUBJECTS + 1000)


BOOTSTRAP_BATCH = 2  # replicates per call; one call is limited to 1e6 simulations


def run_bootstrap(model, subjects, batches):
    """Subject bootstrap in batches with distinct seeds, pooled for percentiles."""
    parameters, omega, error, covariates = full_model_inputs()
    results = [model.bootstrap_population(
        subjects, parameters, omega, error, covariates=covariates,
        replicates=BOOTSTRAP_BATCH, seed=7331 + batch, minimum_success_fraction=0.5,
        max_simulations=1_000_000, **SAEM_OPTIONS) for batch in range(batches)]
    records = [r for result in results for r in result["replicates"]]
    converged = [r for r in records if r["status"] == "converged"]
    assert len(converged) >= 0.8 * len(records), "fewer than 80% of replicates converged"

    def percentiles(values):
        return np.quantile(values, [0.025, 0.5, 0.975]).tolist()

    intervals = {name: percentiles([r["parameters"][name]["value"] for r in converged])
                 for name in ("cl", "v", "ka")}
    omega_values = np.asarray([r["omega"] for r in converged])
    for i, name in enumerate(("cl", "v", "ka")):
        intervals[f"omega_{name}"] = percentiles(omega_values[:, i, i])
    intervals["wt_exponent"] = percentiles(
        [r["covariate_effects"][0]["estimate"] for r in converged])
    for key in ("additive", "proportional"):
        intervals[f"{key}_sd"] = percentiles([r["error"][key] for r in converged])
    pooled = {"batches": batches, "replicates": len(records), "converged": len(converged),
              "seeds": [r["seed"] for r in records],
              "percentiles_2.5_50_97.5": intervals}
    return pooled, results


def check_recovery(est, native):
    failures = []

    def check(label, value, target, tolerance, relative):
        error = abs(value - target) / target if relative else abs(value - target)
        if error > tolerance:
            failures.append(f"{label}: {value:.4g} vs {target:.4g}")

    for name in ("cl", "v", "ka"):
        check(name, est[name], TRUE[name], TOLERANCE[name], True)
    for name in ("cl", "v", "ka"):
        check(f"omega_{name}", est["omega"][name], TRUE["omega"][name], TOLERANCE["omega"], True)
    check("wt_exponent", est["wt_exponent"], TRUE["wt_exponent"], TOLERANCE["wt_exponent"], False)
    check("additive_sd", est["additive_sd"], TRUE["additive_sd"], TOLERANCE["additive_sd"], False)
    check("proportional_sd", est["proportional_sd"], TRUE["proportional_sd"],
          TOLERANCE["proportional_sd"], True)
    for label, fit in native.items():
        check(f"{label}_cl", fit["fixed_effects"]["cl"]["value"], TRUE["cl"],
              TOLERANCE["native_cl"], True)
    assert not failures, "parameter recovery failed: " + "; ".join(failures)


def write_csv(path, rows):
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("population-pk-output"))
    parser.add_argument("--vpc-replicates", type=int, default=200)
    parser.add_argument("--bootstrap-batches", type=int, default=12)
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    clock = time.perf_counter()

    rows, weights, simulated = simulate_cohort()
    write_csv(args.output / "data.csv", rows)
    write_csv(args.output / "simulated-subjects.csv", simulated)

    # Full model: Python SAEM on the prepared records.
    model = qq.CompiledModel(MODEL_SOURCE)
    dataset = prepare(rows, template("saem", ["cl"], {"cl": 2.0, "v": 45.0, "ka": 1.5}),
                      0.1, 0.2)
    fit, subjects = fit_full_model(model, dataset, weights)
    assert fit["status"] == "converged", fit["status"]
    est = estimates(fit)

    # Reduced models: native one-effect fits with V/F, KA and residual error fixed
    # at the full-model estimates.
    fixed = {"v": est["v"], "ka": est["ka"]}
    error = {"additive": est["additive_sd"], "proportional": est["proportional_sd"]}
    native = {
        "native_focei": native_fit(rows, "focei", fixed, error),
        "native_saem": native_fit(rows, "saem", fixed, error),
        "native_saem_wt": native_fit(rows, "saem", fixed, error, weights=weights),
    }
    for label, result in native.items():
        assert result["status"] == "converged", (label, result["status"])

    gof, diagnostics = gof_rows(fit, weights)
    write_csv(args.output / "gof.csv", gof)
    vpc = run_vpc(model, subjects, fit, est, args.vpc_replicates, args.workers)
    assert vpc["status"] == "complete", vpc["status"]
    bootstrap, bootstrap_batches = run_bootstrap(model, subjects, args.bootstrap_batches)

    check_recovery(est, native)

    realized = {name: float(np.var([s[f"ETA_{name.upper()}"] for s in simulated], ddof=1))
                for name in ("cl", "v", "ka")}
    summary = {
        "software": {"qspquest": qq.__version__},
        "design": {"seed": SEED, "subjects": SUBJECTS, "observations": len(gof),
                   "dose_mg": DOSE_MG, "times_h": TIMES_H,
                   "weight_kg_range": [min(weights.values()), max(weights.values())],
                   "weight_kg_median": float(np.median(list(weights.values())))},
        "simulated": {**TRUE, "realized_eta_variance": realized},
        "python_saem": {"status": fit["status"], "iterations": fit["iterations"],
                        "objective_kind": fit["objective_kind"], "estimates": est,
                        "eta_shrinkage": dict(zip(
                            fit["random_effects"],
                            diagnostics["population_diagnostics"]["eta_variance_shrinkage"])),
                        "residual_summary": diagnostics["residual_summary"],
                        "warnings": diagnostics["warnings"]},
        "native": {label: {"status": r["status"], "objective_kind": r["objective_kind"],
                           "cl": r["fixed_effects"]["cl"]["value"], "omega_cl": r["omega"][0][0],
                           "wt_exponent": (r.get("covariate_effect") or {}).get("coefficient")}
                   for label, r in native.items()},
        "vpc": {"status": vpc["status"], "replicates": vpc["replicates"],
                "bins": [{"start": b["start"], "end": b["end"],
                          **{f"p{round(x['percentile'] * 100)}": {
                              "observed": x["observed"], "simulated": x["simulated"],
                              "interval": x["confidence_interval"]} for x in b["quantiles"]}}
                         for b in vpc["bins"]]},
        "bootstrap": bootstrap,
        "elapsed_s": round(time.perf_counter() - clock, 1),
    }
    write_json(args.output / "summary.json", summary)
    for name, value in {"saem-fit.json": fit, "vpc.json": vpc,
                        "bootstrap.json": bootstrap_batches,
                        "native-fits.json": native}.items():
        write_json(args.output / name, value)
    (args.output / "model.qsp").write_text(MODEL_SOURCE, encoding="utf-8")
    print(json.dumps({k: summary[k] for k in ("python_saem", "native", "bootstrap")},
                     indent=2))
    print(f"recovery checks passed; outputs in {args.output}")


if __name__ == "__main__":
    main()
