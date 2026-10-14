# QSPQuest synthetic user-guide example. Run in a directory for analysis outputs.
import copy
import csv
import json
import math
from pathlib import Path

import qspquest
from qspquest import (
    CompiledModel,
    CompiledSensitivities,
    QSPQuestError,
    format_model,
    population_fit_dataset,
)

source = """model quickstart_one_compartment
time_unit = h
parameters
  cl = 0.3 [L/h] log
  v = 3 [L] log
states
  central = 0 [mg]
dynamics
  d/dt(central) = input_central - cl / v * central
outputs
  cp = central / v [mg/L]
dosing
  central [mg] = 1
end
"""

model = CompiledModel(source)
print(model.output_names, model.output_units)


def q(value, unit):
    return {"value": value, "unit": unit}


times = [0, 1, 2, 4, 8, 12]
request = {
    "schema": "qspquest.run/v0.1",
    "solver": "diffsol_bdf",
    "budgets": {"solver_callbacks": 1_000_000, "events": 100, "output_values": 10_000},
    "regimen": {
        "end": q(12, "h"),
        "samples": [],
        "observations": [{"time": q(t, "h"), "side": "post"} for t in times],
        "administrations": [
            {
                "target": "central",
                "time": q(0, "h"),
                "amount": q(6, "mg"),
                "delivery": {"kind": "bolus"},
                "bioavailability": 1.0,
            }
        ],
        "rtol": 1e-10,
        "atol": 1e-12,
    },
}
result = model.run(request)
cp = next(column for column in result["outputs"] if column["name"] == "cp")
assert result["times"] == times
assert result["sides"] == ["post"] * len(times)
for t, value in zip(result["times"], cp["values"]):
    assert abs(value - 2 * math.exp(-0.1 * t)) < 1e-8
print(list(zip(result["times"], cp["values"])))


def result_rows(answer):
    return [
        {
            "TIME": t,
            "SIDE": side,
            "OUTPUT": column["name"],
            "VALUE": value,
            "UNIT": column["unit"],
        }
        for column in answer["outputs"]
        for t, side, value in zip(answer["times"], answer["sides"], column["values"])
    ]


table = result_rows(result)
with open("trajectory.csv", "w", newline="", encoding="utf-8") as handle:
    writer = csv.DictWriter(handle, fieldnames=list(table[0]))
    writer.writeheader()
    writer.writerows(table)

changed = copy.deepcopy(request)
changed["parameters"] = {"cl": q(0.6, "L/h"), "v": q(3, "L")}
faster_clearance = model.run(changed)

repeated = copy.deepcopy(request)
repeated["regimen"]["administrations"][0]["repeat"] = {
    "interval": q(4, "h"),
    "additional": 2,
}
repeated_result = model.run(repeated)  # Doses at 0, 4, and 8 h.

infusion = copy.deepcopy(request)
infusion["regimen"]["administrations"][0]["delivery"] = {
    "kind": "infusion",
    "span": {"kind": "duration", "duration": q(2, "h")},
}
infusion_result = model.run(infusion)  # 6 mg over 2 h: 3 mg/h before F.

oral_source = """model quickstart_oral_two_compartment
time_unit = h
parameters
  cl = 1 [L/h] log
  vc = 5 [L] log
  vp = 10 [L] log
  exchange = 0.5 [L/h] log
  ka = 1 [1/h] log
states
  depot = 0 [mg]
  central = 0 [mg]
  peripheral = 0 [mg]
dynamics
  d/dt(depot) = -ka * depot
  d/dt(central) = ka * depot - cl / vc * central - exchange * (central / vc - peripheral / vp)
  d/dt(peripheral) = exchange * (central / vc - peripheral / vp)
outputs
  cp = central / vc [mg/L]
  depot_amount = depot [mg]
  central_amount = central [mg]
  peripheral_amount = peripheral [mg]
dosing
  depot [mg] bolus = 1
end
"""
oral_model = CompiledModel(oral_source)
oral_request = copy.deepcopy(request)
oral_dose = oral_request["regimen"]["administrations"][0]
oral_dose["target"] = "depot"
oral_dose["amount"] = q(100, "mg")
oral_dose["bioavailability"] = 0.8
oral_result = oral_model.run(oral_request)
oral_columns = {column["name"]: column for column in oral_result["outputs"]}
assert oral_columns["cp"]["values"][0] == 0
assert oral_columns["depot_amount"]["values"][0] == 80
assert all(
    value >= -1e-8 for column in oral_result["outputs"] for value in column["values"]
)

at_dose = copy.deepcopy(request)
at_dose["regimen"]["observations"] = [
    {"time": q(0, "h"), "side": "pre"},
    {"time": q(0, "h"), "side": "post"},
]
event_result = model.run(at_dose)
assert event_result["outputs"][0]["values"] == [0.0, 2.0]

scan_request = {
    "schema": "qspquest.scan/v0.1",
    "run": copy.deepcopy(request),
    "points": [
        {"id": f"cl_{value}", "parameters": {"cl": q(value, "L/h")}}
        for value in [0.3, 0.6, 1.2]
    ],
    "total_budgets": {
        "solver_callbacks": 3_000_000,
        "events": 300,
        "output_values": 30_000,
    },
}
scan = model.scan(scan_request)
for point in scan["results"]:
    print(point["id"], point["parameters"], point["result"]["outputs"][0]["values"][-1])

sensitivity_request = {
    "schema": "qspquest.sensitivity/v0.1",
    "run": copy.deepcopy(request),
    "with_respect_to": ["cl", "v"],
    "derivative_absolute_tolerances": {
        "cl": {"central": q(1e-12, "mg.h/L")},
        "v": {"central": q(1e-12, "mg/L")},
    },
}
sensitive = CompiledSensitivities(source, ["cl", "v"])
sensitivities = sensitive.run(sensitivity_request)
for column in sensitivities["derivatives"]:
    print(column["parameter"], column["output"], column["unit"])

morris_request = {
    "schema": "qspquest.morris/v0.1",
    "problem": {
        "kind": "generated",
        "request": {
            "run": copy.deepcopy(request),
            "ranges": [
                {"parameter": "cl", "lower": q(0.1, "L/h"), "upper": q(0.7, "L/h")},
                {"parameter": "v", "lower": q(3, "L"), "upper": q(6, "L")},
            ],
            "trajectory_count": 2,
            "num_levels": 4,
            "seed": 42,
            "total_budgets": {
                "solver_callbacks": 6_000_000,
                "events": 600,
                "output_values": 60_000,
            },
            "output": "cp",
            "row": 3,  # Zero-based: our selected 4 h row.
        },
    },
}
morris = model.morris(morris_request)
for effect in morris["result"]["effects"]:
    print(effect["parameter"], effect["mu_star"], effect["sigma"])

observations = [
    {
        "row": row,
        "output": "cp",
        "value": q(6 / 4.5 * math.exp(-0.6 * t / 4.5), "mg/L"),
        "error": {"additive_sd": q(0.1, "mg/L"), "proportional_sd": 0.0},
    }
    for row, t in enumerate(times)
]
fit_request = {
    "schema": "qspquest.fit/v0.1",
    "problem": {
        "kind": "individual",
        "request": {
            "objective": {
                "simulation": copy.deepcopy(sensitivity_request),
                "observations": observations,
                "priors": [],
            },
            "bounds": [
                {"parameter": "cl", "lower": q(0.05, "L/h"), "upper": q(1.2, "L/h")},
                {"parameter": "v", "lower": q(1, "L"), "upper": q(8, "L")},
            ],
            "max_evaluations": 400,
            "max_iterations": 100,
            "gradient_tolerance": 1e-5,
        },
    },
}
fitted = sensitive.fit(fit_request)
fit = fitted["fit"]
print(fit["status"], fit["parameters"])
assert fit["status"] == "converged", fit
assert abs(fit["parameters"]["cl"]["value"] - 0.6) < 1e-4
assert abs(fit["parameters"]["v"]["value"] - 4.5) < 1e-4

replay = copy.deepcopy(request)
replay["parameters"] = fit["parameters"]
fit_predictions = model.run(replay)

scalar_run = copy.deepcopy(request)
scalar_run["parameters"] = {"v": q(4.5, "L")}
scalar_request = {
    "schema": "qspquest.scalar-fit/v0.1",
    "run": scalar_run,
    "parameter": "cl",
    "lower": q(0.1, "L/h"),
    "upper": q(1, "L/h"),
    "observations": copy.deepcopy(observations),
    "max_iterations": 80,
    "absolute_tolerance": 1e-7,
}
scalar = model.fit_scalar(scalar_request)
print(scalar["status"], scalar["parameters"], scalar["evaluations"])
assert scalar["status"] == "converged"
assert abs(scalar["parameters"]["cl"]["value"] - 0.6) < 1e-4

rows = []
for index, eta in enumerate([-0.22, -0.08, 0.08, 0.22], start=1):
    subject_id = f"S{index}"
    rows.append({"ID": subject_id, "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6, "MDV": 1})
    for t in [1, 2, 4]:
        rows.append(
            {
                "ID": subject_id,
                "TIME": t,
                "EVID": 0,
                "CMT": 2,
                "AMT": 0,
                "MDV": 0,
                "DV": 6 / 4.5 * math.exp(-0.6 * math.exp(eta) * t / 4.5),
            }
        )

pop_sensitivity = copy.deepcopy(sensitivity_request)
pop_sensitivity["with_respect_to"] = ["cl"]
pop_sensitivity["derivative_absolute_tolerances"].pop("v")
pop_run = pop_sensitivity["run"]
pop_run["parameters"] = {"cl": q(0.5, "L/h"), "v": q(4.5, "L")}
pop_run["regimen"]["samples"] = []
pop_run["regimen"].pop("observations")
pop_run["regimen"]["administrations"] = []

population_template = {
    "schema": "qspquest.fit/v0.1",
    "problem": {
        "kind": "focei",
        "request": {
            "subjects": [
                {"simulation": pop_sensitivity, "observations": [], "priors": []}
            ],
            "fixed_effects": [
                {"parameter": "cl", "lower": q(0.35, "L/h"), "upper": q(0.85, "L/h")}
            ],
            "random_effects": [{"parameter": "cl"}],
            "omega": [[0.04]],
            "omega_diagonal_bounds": [[0.04, 0.04]],
            "eta_bound": 1.5,
            "max_evaluations": 30_000,
            "max_iterations": 80,
            "gradient_tolerance": 0.005,
            "seed": 17,
        },
    },
}
endpoints = {
    2: {
        "output": "cp",
        "value_unit": "mg/L",
        "error": {
            "additive_sd": q(0.05, "mg/L"),
            "proportional_sd": 0.0,
        },
    }
}
dataset = population_fit_dataset(
    rows,
    population_template,
    compartments={1: "central"},
    endpoints=endpoints,
    time_unit="h",
    amount_unit="mg",
    observation_side="post",
)
population_model = CompiledSensitivities(source, ["cl"])
population_fit = population_model.fit_population_data(dataset)
population = population_fit["result"]["population"]
print(population["status"], population["objective_kind"])
print(population["fixed_effects"], population["omega"])
assert population["status"] == "converged"
assert abs(population["fixed_effects"]["cl"]["value"] - 0.6) < 0.06

# Requires pandas; replace rows with your own validated data.
# rows = pd.read_csv("observations.csv", dtype={"ID": "string"})
# Check TIME/AMT/DV numeric types, CMT mappings, MDV and units before fitting.

saem_template = copy.deepcopy(population_template)
saem_template["problem"]["kind"] = "saem"
saem_template["problem"]["request"]["max_iterations"] = 8
saem_dataset = population_fit_dataset(
    rows,
    saem_template,
    compartments={1: "central"},
    endpoints=endpoints,
    time_unit="h",
    amount_unit="mg",
    observation_side="post",
)
saem_fit = population_model.fit_population_data(saem_dataset)
saem = saem_fit["result"]["population"]
print(saem["status"], saem["objective_kind"])

bundle = Path("qspquest-analysis")
bundle.mkdir(exist_ok=True)
(bundle / "model.qsp").write_text(source, encoding="utf-8")
(bundle / "model.canonical.qsp").write_text(format_model(source), encoding="utf-8")
artifacts = {
    "run.request.json": request,
    "run.result.json": result,
    "fit.request.json": fit_request,
    "fit.result.json": fitted,
    "population.request.json": dataset.request,
    "population.result.json": population_fit,
    "population.rows.json": rows,
    "software.json": {"qspquest": qspquest.__version__},
}
for filename, value in artifacts.items():
    (bundle / filename).write_text(
        json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8"
    )
print(result["identity"])

invalid = copy.deepcopy(request)
invalid["parameters"] = {"cl": q(1, "h")}
try:
    model.run(invalid)
except QSPQuestError as error:
    print(error.code, str(error))
    print(error.diagnostic)
else:
    raise AssertionError("incompatible clearance units were accepted")
assert model.run(request) == result  # The failed request did not alter the model.
