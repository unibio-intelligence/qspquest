"""Single-dose pharmacokinetics of a synthetic drug after IV bolus, IV infusion and oral doses.

Simulated concentrations are compared with the analytical solutions, and the
analytic solver is compared with the BDF solver. Outputs are written to
``--out`` (default ``single-dose-output``).
"""

import argparse
import csv
import json
import math
from pathlib import Path

import qspquest as qq

MODEL = """model synthetic_route_comparison
 time_unit = h
 description = "Synthetic one-compartment model with first-order absorption"
parameters
 cl = 2.4 [L/h] log estimated
 v = 60 [L] log estimated
 ka = 0.9 [1/h] log estimated
linear_pk
 amount_unit = mg
 clearance = cl
 compartment central volume v = 0 [mg]
 depot depot absorption ka = 0 [mg]
outputs
 cp = central / v [mg/L]
end
"""

CL, V, KA = 2.4, 60.0, 0.9  # L/h, L, 1/h
K = CL / V  # elimination rate constant, 1/h
F = 0.7  # oral bioavailability
DOSE = 300.0  # mg
INFUSION_HOURS = 0.75
TIMES = [0, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8, 12, 24, 48, 72]
SPARSE = [0, 1, 2, 4, 8, 24, 48, 72]
DENSE = [i * 0.05 for i in range(1441)]
TOLERANCE = 1e-9  # mg/L


def bolus_cp(t):
    return DOSE / V * math.exp(-K * t)


def infusion_cp(t):
    css = DOSE / INFUSION_HOURS / CL  # steady-state concentration of this infusion rate
    if t <= INFUSION_HOURS:
        return css * (1 - math.exp(-K * t))
    return css * (1 - math.exp(-K * INFUSION_HOURS)) * math.exp(-K * (t - INFUSION_HOURS))


def oral_cp(t, ka=KA):
    return F * DOSE * ka / (V * (ka - K)) * (math.exp(-K * t) - math.exp(-ka * t))


def oral_auc(t, ka=KA):
    """Integral of the oral concentration from 0 to t."""
    scale = F * DOSE * ka / (V * (ka - K))
    return scale * ((1 - math.exp(-K * t)) / K - (1 - math.exp(-ka * t)) / ka)


ROUTES = {
    "IV bolus": (lambda: qq.bolus("central", DOSE, "mg"), bolus_cp),
    "IV infusion": (lambda: qq.infusion("central", DOSE, "mg", duration=INFUSION_HOURS),
                    infusion_cp),
    "Oral": (lambda: qq.bolus("depot", DOSE, "mg", bioavailability=F), oral_cp),
}


def request(administration, times, solver="analytic_linear_pk", parameters=None, end=72):
    return qq.run_request(end=end, times=times, administrations=[administration],
                          solver=solver, parameters=parameters)


def cp(result):
    column = next(c for c in result["outputs"] if c["name"] == "cp")
    assert column["unit"] == "mg/L"
    return column["values"]


def main(out):
    out.mkdir(parents=True, exist_ok=False)

    # A clearance declared in litres is dimensionally inconsistent.
    try:
        qq.CompiledModel(MODEL.replace("cl = 2.4 [L/h]", "cl = 2.4 [L]"))
        raise AssertionError("a clearance in litres must not compile")
    except qq.QSPQuestError as error:
        d = error.diagnostic
        print(f"Model with cl in L rejected: {d['message']} (line {d['line']}, "
              f"column {d['column']})")
    model = qq.CompiledModel(MODEL)

    print("\nRoute         Cmax (mg/L)  Tmax (h)  AUC0-72 (mg*h/L)")
    rows, summaries, max_analytical, max_bdf = [], {}, 0.0, 0.0
    for route, (dose, exact) in ROUTES.items():
        result = model.run(request(dose(), TIMES))
        bdf = model.run(request(dose(), TIMES, solver="diffsol_bdf"))
        for t, value, numeric in zip(result["times"], cp(result), cp(bdf)):
            max_analytical = max(max_analytical, abs(value - exact(t)))
            max_bdf = max(max_bdf, abs(value - numeric))
            rows.append({"route": route, "time_h": t, "cp_mg_per_L": value,
                         "analytical_mg_per_L": exact(t), "bdf_mg_per_L": numeric})
        s = qq.exposure_summary(result, "cp")
        summaries[route] = {key: s[key] for key in ("cmax", "tmax", "auc", "auc_unit")}
        print(f"{route:12s} {s['cmax']:12.3f} {s['tmax']:9.2f} {s['auc']:17.2f}")
    assert max_analytical < TOLERANCE and max_bdf < 1e-7
    print(f"Maximum difference from the analytical solution: {max_analytical:.1e} mg/L "
          f"({len(rows)} concentrations)")
    print(f"Maximum difference between analytic and BDF solutions: {max_bdf:.1e} mg/L")

    auc_inf = {"IV": DOSE / CL, "Oral": F * DOSE / CL}
    print(f"\nAUC0-inf (analytical): IV {auc_inf['IV']:.1f} mg*h/L, oral "
          f"{auc_inf['Oral']:.1f} mg*h/L; CL/F = {CL / F:.3f} L/h")

    tmax = math.log(KA / K) / (KA - K)
    print("\nOral dose: effect of the sampling schedule")
    print("Schedule              Tmax (h)  Cmax (mg/L)  AUC0-72 (mg*h/L)  Error (%)")
    design = {}
    for label, times in {"8 samples": SPARSE, "15 samples": TIMES,
                         "every 0.05 h": DENSE}.items():
        s = qq.exposure_summary(model.run(request(ROUTES["Oral"][0](), times)), "cp")
        error = 100 * (s["auc"] - oral_auc(72)) / oral_auc(72)
        design[label] = {"tmax": s["tmax"], "cmax": s["cmax"], "auc": s["auc"],
                         "auc_error_percent": error}
        print(f"{label:20s} {s['tmax']:9.2f} {s['cmax']:12.3f} {s['auc']:17.2f} "
              f"{round(error, 2) + 0.0:10.2f}")
    assert abs(design["every 0.05 h"]["auc_error_percent"]) < 0.01
    print(f"{'Analytical':20s} {tmax:9.2f} {oral_cp(tmax):12.3f} {oral_auc(72):17.2f}")

    # Extended-release formulation: KA is smaller than k.
    slow = 0.02
    windows = [(24, 48), (96, 120), (288, 312)]
    times = sorted({t for window in windows for t in window})
    result = model.run(request(ROUTES["Oral"][0](), times, end=312,
                               parameters={"ka": qq.quantity(slow, "1/h")}))
    late = dict(zip(result["times"], cp(result)))
    assert all(abs(value - oral_cp(t, slow)) < TOLERANCE for t, value in late.items())
    slope = {f"{a}-{b} h": math.log(late[b] / late[a]) / (b - a) for a, b in windows}
    print(f"\nExtended release (KA = {slow} 1/h, k = {K:.2f} 1/h): slope of ln C, "
          + ", ".join(f"{w} {v:.4f}" for w, v in slope.items()) + " 1/h")

    with (out / "concentrations.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    (out / "model.qsp").write_text(MODEL)
    (out / "summary.json").write_text(json.dumps(
        {"routes": summaries, "sampling_schedules": design, "auc_0_inf": auc_inf,
         "half_life_h": math.log(2) / K, "extended_release_slopes_per_h": slope},
        indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("single-dose-output"))
    main(parser.parse_args().out)
