"""Noncompartmental analysis of simulated single-dose profiles of a synthetic drug."""

import math
import random

import qspquest as qq

SOURCE = """model synthetic_nca_drug
time_unit = h
parameters
  cl = 2.4 [L/h] log
  v = 60 [L] log
  ka = 0.9 [1/h] log
states
  depot = 0 [mg]
  central = 0 [mg]
dynamics
  d/dt(depot) = -ka * depot
  d/dt(central) = ka * depot - cl / v * central
outputs
  cp = central / v [mg/L]
dosing
  depot [mg] bolus = 1
  central [mg] bolus = 1
end
"""

CL, V, KA, F, DOSE = 2.4, 60.0, 0.9, 0.7, 300.0
K = CL / V
TIMES = [0.25, 0.5, 1, 1.5, 2, 3, 4, 6, 8, 12, 24, 36, 48, 72]
UNITS = {"time": "h", "concentration": "mg/L", "amount": "mg"}
LLOQ = 0.1
BLQ_RULE = {"leading": "zero", "embedded": "error", "trailing": "exclude"}


def simulate(model, target, bioavailability, times, parameters=None):
    request = qq.run_request(
        end=times[-1],
        times=times,
        administrations=[qq.bolus(target, DOSE, "mg", bioavailability=bioavailability)],
        parameters=parameters,
        rtol=1e-10,
        atol=1e-12,
    )
    result = model.run(request)
    cp = next(column for column in result["outputs"] if column["name"] == "cp")
    return cp["values"]


def oral_closed_form(t):
    return F * DOSE * KA / (V * (KA - K)) * (math.exp(-K * t) - math.exp(-KA * t))


def profile_rows(subject, times, values, lloq=None):
    rows = [{"subject": subject, "time": 0.0, "blq": True}]
    for t, c in zip(times, values):
        if lloq is not None and c < lloq:
            rows.append({"subject": subject, "time": t, "blq": True})
        else:
            rows.append({"subject": subject, "time": t, "concentration": c})
    return rows


def simulated_subjects(model):
    """Eight oral profiles with variability, residual error and BLQ rows."""
    rng = random.Random(2026)
    times = TIMES + [96]
    observations, doses, inputs = [], [], {}
    for index in range(1, 9):
        individual = {
            "cl": {"value": CL * math.exp(rng.gauss(0, 0.25)), "unit": "L/h"},
            "v": {"value": V * math.exp(rng.gauss(0, 0.15)), "unit": "L"},
            "ka": {"value": KA * math.exp(rng.gauss(0, 0.4)), "unit": "1/h"},
        }
        values = [
            c * (1 + rng.gauss(0, 0.1))
            for c in simulate(model, "depot", F, times, individual)
        ]
        observations += profile_rows(index, times, values, LLOQ)
        doses.append({"subject": index, "amount": DOSE, "route": "extravascular"})
        cl, v = individual["cl"]["value"], individual["v"]["value"]
        inputs[index] = (math.log(2) * v / cl, cl / F)
    return observations, doses, inputs


def parameters_of(result):
    return {code: item["value"] for code, item in result["profiles"][0]["parameters"].items()}


def table(header, rows):
    print("| " + " | ".join(header) + " |")
    print("| " + " | ".join("---" if i == 0 else "---:" for i in range(len(header))) + " |")
    for row in rows:
        print("| " + " | ".join(row) + " |")
    print()


def main():
    model = qq.CompiledModel(SOURCE)

    # Section 1: simulated oral profile compared with closed-form values.
    oral = simulate(model, "depot", F, TIMES)
    error = max(abs(c - oral_closed_form(t)) for t, c in zip(TIMES, oral))
    print(f"Maximum |simulated - analytical| oral concentration: {error:.1e} mg/L\n")
    dose = [{"subject": "oral", "amount": DOSE, "route": "extravascular"}]
    rows = profile_rows("oral", TIMES, oral)
    linear = parameters_of(
        qq.noncompartmental_analysis(rows, dose, units=UNITS, auc_method="linear")
    )
    logdown = parameters_of(qq.noncompartmental_analysis(rows, dose, units=UNITS))
    tmax = math.log(KA / K) / (KA - K)
    auc72 = F * DOSE / CL * (1 - (KA * math.exp(-K * 72) - K * math.exp(-KA * 72)) / (KA - K))
    analytical = {
        "CMAX": oral_closed_form(tmax), "TMAX": tmax, "AUCLST": auc72,
        "LAMZ": K, "LAMZHL": math.log(2) / K, "AUCIFO": F * DOSE / CL,
        "AUCIFP": F * DOSE / CL, "CLFO": CL / F, "VZFO": V / F,
        "AUCPEO": 100 * oral_closed_form(72) / K / (F * DOSE / CL),
    }
    labels = [
        ("CMAX", "Cmax (mg/L)", 3), ("TMAX", "Tmax (h)", 2),
        ("AUCLST", "AUClast (mg·h/L)", 2), ("LAMZ", "λz (1/h)", 5),
        ("LAMZHL", "t½ (h)", 2), ("AUCIFO", "AUCinf,obs (mg·h/L)", 2),
        ("AUCIFP", "AUCinf,pred (mg·h/L)", 2), ("AUCPEO", "AUC%extrap,obs (%)", 2),
        ("CLFO", "CL/F (L/h)", 3), ("VZFO", "Vz/F (L)", 2),
    ]
    table(
        ["Parameter", "Linear", "Linear-up/log-down", "Analytical"],
        [
            [name, f"{linear[code]:.{d}f}", f"{logdown[code]:.{d}f}",
             f"{analytical[code]:.{d}f}" if code in analytical else "—"]
            for code, name, d in labels
        ],
    )
    print(
        f"λz points: {logdown['LAMZNPT']} ({logdown['LAMZLL']:g} to "
        f"{logdown['LAMZUL']:g} h); adjusted R² {logdown['R2ADJ']:.6f}\n"
    )
    strict = parameters_of(
        qq.noncompartmental_analysis(
            rows, dose, units=UNITS, lambda_z={"rule": "adjusted_r2", "tolerance": 0.0}
        )
    )
    print(
        f"Tolerance 0: λz points {strict['LAMZNPT']} ({strict['LAMZLL']:g} to "
        f"{strict['LAMZUL']:g} h); λz {strict['LAMZ']:.5f} 1/h\n"
    )

    # Section 2: simulated IV bolus profile.
    iv = simulate(model, "central", 1.0, TIMES)
    result = qq.noncompartmental_analysis(
        [{"subject": "iv", "time": t, "concentration": c} for t, c in zip(TIMES, iv)],
        [{"subject": "iv", "amount": DOSE, "route": "iv_bolus"}],
        units=UNITS,
    )
    p = parameters_of(result)
    table(
        ["Parameter", "NCA", "Analytical"],
        [
            ["C0 (mg/L)", f"{p['C0']:.4f}", f"{DOSE / V:.4f}"],
            ["λz (1/h)", f"{p['LAMZ']:.5f}", f"{K:.5f}"],
            ["AUCinf,obs (mg·h/L)", f"{p['AUCIFO']:.3f}", f"{DOSE / CL:.3f}"],
            ["CL (L/h)", f"{p['CLO']:.4f}", f"{CL:.4f}"],
            ["Vz (L)", f"{p['VZO']:.3f}", f"{V:.3f}"],
        ],
    )

    # Section 3: eight simulated subjects with variability and an LLOQ.
    observations, doses, inputs = simulated_subjects(model)
    try:
        qq.noncompartmental_analysis(observations, doses, units=UNITS)
    except qq.QSPQuestError as failure:
        print(f"Without a BLQ rule: {failure}\n")
    panel = qq.noncompartmental_analysis(observations, doses, units=UNITS, blq=BLQ_RULE)
    rows = []
    for profile in panel["profiles"]:
        q = {code: item["value"] for code, item in profile["parameters"].items()}
        excluded = sum(row["role"] == "trailing_blq_excluded" for row in profile["rows"])
        rows.append([
            str(profile["subject"]), f"{q['CMAX']:.2f}", f"{q['TMAX']:g}",
            f"{q['TLST']:g}", str(excluded), f"{q['AUCLST']:.1f}",
            f"{q['LAMZNPT']} ({q['LAMZLL']:g}–{q['LAMZUL']:g})", f"{q['R2ADJ']:.4f}",
            f"{q['LAMZHL']:.1f}", f"{inputs[profile['subject']][0]:.1f}",
            f"{q['AUCPEO']:.1f}", f"{q['CLFO']:.2f}",
            f"{inputs[profile['subject']][1]:.2f}", f"{q['VZFO']:.1f}",
        ])
    table(
        ["Subject", "Cmax", "Tmax", "Tlast", "BLQ excluded", "AUClast",
         "λz points (h)", "Adj. R²", "t½", "Input t½", "AUC%extrap", "CL/F",
         "Input CL/F", "Vz/F"],
        rows,
    )


if __name__ == "__main__":
    main()
