"""Direct, effect-compartment and indirect response models for a synthetic oral drug.

Each simulation is compared with a closed-form solution or an independent
scipy ODE solution. The script prints a summary and exits with status 1 if a
comparison fails. Requires NumPy and SciPy for the reference solutions.
"""

import argparse
import json
import math
import random
from pathlib import Path

import numpy as np
from scipy.integrate import solve_ivp
from scipy.optimize import brentq

import qspquest as qq

MODELS = Path(__file__).resolve().parent / "pk-pd-response"

# Synthetic drug and response parameters (declared independently of the .qsp files).
CL, V, KA, F = 2.4, 60.0, 0.9, 0.7
K = CL / V
EMAX, EC50, HILL = 80.0, 2.5, 1.5
KE0 = 0.15
R0, KOUT, IMAX, IC50, SMAX, SC50 = 120.0, 0.08, 0.9, 1.2, 2.5, 1.8
KIN = R0 * KOUT

# Comparison bounds: abs(a - b) <= absolute + relative * max(abs(a), abs(b)).
BOUNDS = {
    "cp": (1e-9, 1e-7),
    "ce": (1e-9, 1e-7),
    "effect": (1e-7, 1e-7),
    "r_kin": (1e-6, 1e-7),
    "r_kout": (1e-6, 1e-7),
}
CHECKS = []


def model(name):
    return qq.CompiledModel((MODELS / name).read_text())


def schedule(amount, interval, count):
    return [(i * interval, amount) for i in range(count)]


def oral_doses(amount, interval, count):
    dose = qq.bolus("depot", amount, "mg")
    if count > 1:
        dose["repeat"] = {"interval": qq.quantity(interval, "h"), "additional": count - 1}
    return [dose]


def simulate(compiled, times, administrations, end=None):
    request = qq.run_request(
        end=end or times[-1], times=list(times), administrations=administrations,
        rtol=1e-10, atol=1e-12,
    )
    result = compiled.run(request)
    return {c["name"]: np.asarray(c["values"]) for c in result["outputs"]}


def compare(label, output, engine, reference):
    absolute, relative = BOUNDS[output]
    engine, reference = np.asarray(engine), np.asarray(reference)
    scale = absolute + relative * np.maximum(np.abs(engine), np.abs(reference))
    worst = float(np.max(np.abs(engine - reference) / scale))
    CHECKS.append({"check": f"{label}: {output}", "max_scaled_error": worst, "passed": worst <= 1})


def cp_analytic(t, doses):
    t = np.asarray(t, dtype=float)
    total = np.zeros_like(t)
    for start, amount in doses:
        s = np.clip(t - start, 0, None)
        total += np.where(t > start,
                          F * amount * KA / (V * (KA - K)) * (np.exp(-K * s) - np.exp(-KA * s)), 0)
    return total


def ce_analytic(t, doses):
    t = np.asarray(t, dtype=float)
    rates = (K, KA, KE0)
    total = np.zeros_like(t)
    for start, amount in doses:
        s = np.clip(t - start, 0, None)
        part = sum(np.exp(-r * s) / np.prod([o - r for o in rates if o != r]) for r in rates)
        total += np.where(t > start, F * amount * KA * KE0 / V * part, 0)
    return total


def emax(c):
    x = (np.asarray(c) / EC50) ** HILL
    return EMAX * x / (1 + x)


def idr_reference(times, doses, end):
    """Independent four-state ODE solution; depot doses are applied as jumps."""
    def rhs(_, y):
        depot, central, r_kin, r_kout = y
        c = central / V
        return [-KA * depot, KA * depot - K * central,
                KIN * (1 - IMAX * c / (IC50 + c)) - KOUT * r_kin,
                KIN - KOUT * (1 + SMAX * c / (SC50 + c)) * r_kout]

    times = np.asarray(times, dtype=float)
    out = np.empty((len(times), 4))
    y = np.array([0.0, 0.0, R0, R0])
    edges = sorted({d[0] for d in doses} | {end})
    for i, start in enumerate(edges[:-1]):
        y[0] += F * sum(a for t0, a in doses if t0 == start)
        stop = edges[i + 1]
        inside = (times >= start) & ((times < stop) | ((stop == end) & (times <= end)))
        points = np.unique(np.append(times[inside], stop))
        solution = solve_ivp(rhs, (start, stop), y, method="DOP853", rtol=1e-12, atol=1e-12,
                             t_eval=points)
        out[inside] = solution.y.T[np.searchsorted(points, times[inside])]
        y = solution.y[:, -1].copy()
    return {"cp": out[:, 1] / V, "r_kin": out[:, 2], "r_kout": out[:, 3]}


def peak(times, values, lowest=False):
    i = int(np.argmin(values) if lowest else np.argmax(values))
    return float(times[i]), float(values[i])


def direct_emax():
    times = np.round(np.arange(0, 48.0001, 0.05), 10)
    doses = schedule(300, 24, 1)
    sim = simulate(model("direct-emax.qsp"), times, oral_doses(300, 24, 1))
    compare("direct Emax, single dose", "cp", sim["cp"], cp_analytic(times, doses))
    compare("direct Emax, single dose", "effect", sim["effect"], emax(cp_analytic(times, doses)))
    tmax, cmax = peak(times, sim["cp"])
    tpeak, epeak = peak(times, sim["effect"])
    rows = [{"time_h": float(t), "cp_mg_per_L": float(c), "effect_percent": float(e)}
            for t, c, e in zip(times, sim["cp"], sim["effect"]) if t in (1, 2, 4, 8, 12, 24)]
    return {"cmax_mg_per_L": cmax, "tmax_h": tmax, "effect_peak_percent": epeak,
            "effect_peak_time_h": tpeak,
            "tmax_analytic_h": math.log(KA / K) / (KA - K), "table": rows}


def effect_compartment():
    compiled = model("effect-compartment.qsp")
    doses = schedule(300, 24, 1)
    times = np.round(np.arange(0, 96.0001, 0.05), 10)
    sim = simulate(compiled, times, oral_doses(300, 24, 1))
    cp, ce = cp_analytic(times, doses), ce_analytic(times, doses)
    compare("effect compartment, single dose", "cp", sim["cp"], cp)
    compare("effect compartment, single dose", "ce", sim["ce"], ce)
    compare("effect compartment, single dose", "effect", sim["effect"], emax(ce))
    # Signed area of the (cp, effect) loop; positive means counterclockwise.
    x, y = sim["cp"], sim["effect"]
    area = 0.5 * float(np.sum(x * np.roll(y, -1) - np.roll(x, -1) * y))
    # Times at which cp equals 2 mg/L on the rising and falling limbs.
    tmax = math.log(KA / K) / (KA - K)
    level = 2.0
    def gap(t):
        return float(cp_analytic([t], doses)[0]) - level
    rise, fall = brentq(gap, 0.01, tmax, xtol=1e-12), brentq(gap, tmax, 96, xtol=1e-12)
    paired = simulate(compiled, [rise, fall], oral_doses(300, 24, 1), end=96)
    compare("effect compartment, matched cp", "cp", paired["cp"], [level, level])
    compare("effect compartment, matched cp", "effect", paired["effect"],
            emax(ce_analytic([rise, fall], doses)))
    return {
        "ke0_per_h": KE0, "t_half_ke0_h": math.log(2) / KE0,
        "tmax_cp_h": peak(times, sim["cp"])[0], "tmax_ce_h": peak(times, sim["ce"])[0],
        "effect_peak_percent": peak(times, sim["effect"])[1],
        "direct_effect_peak_percent": float(np.max(emax(sim["cp"]))),
        "matched_cp_mg_per_L": level,
        "rising_limb": {"time_h": rise, "ce_mg_per_L": float(paired["ce"][0]),
                        "effect_percent": float(paired["effect"][0])},
        "falling_limb": {"time_h": fall, "ce_mg_per_L": float(paired["ce"][1]),
                         "effect_percent": float(paired["effect"][1])},
        "direct_effect_at_matched_cp_percent": float(emax(level)),
        "loop_signed_area": area,
        "loop_direction": "counterclockwise" if area > 0 else "clockwise",
    }


def indirect_response():
    compiled = model("indirect-response.qsp")
    times = np.round(np.arange(0, 120.0001, 0.05), 10)
    baseline = simulate(compiled, times, [])
    for name in ("r_kin", "r_kout"):
        compare("indirect response, no dose", name, baseline[name], np.full(len(times), R0))
    single = {}
    for amount in (150, 300, 600):
        doses = schedule(amount, 24, 1)
        sim = simulate(compiled, times, oral_doses(amount, 24, 1))
        ref = idr_reference(times, doses, times[-1])
        for name in ("cp", "r_kin", "r_kout"):
            compare(f"indirect response, single {amount} mg", name, sim[name], ref[name])
        single[f"{amount}_mg"] = {
            name: dict(zip(("nadir_time_h", "nadir_nmol_per_L"), peak(times, sim[name], lowest=True)))
            for name in ("r_kin", "r_kout")}
        single[f"{amount}_mg"]["tmax_cp_h"] = peak(times, sim["cp"])[0]
    # Constant concentration: bolus into central plus a matching infusion.
    level, hours = 3.0, 240.0
    constant = [qq.bolus("central", level * V, "mg"),
                qq.infusion("central", CL * level * hours, "mg", duration=hours)]
    steady = simulate(compiled, [hours / 2, hours], constant, end=hours)
    compare("indirect response, constant cp", "cp", steady["cp"], [level, level])
    limits = {"r_kin": R0 * (1 - IMAX * level / (IC50 + level)),
              "r_kout": R0 / (1 + SMAX * level / (SC50 + level))}
    for name, value in limits.items():
        compare("indirect response, constant cp at 240 h", name, steady[name][-1:], [value])
    return {"single_dose": single, "constant_cp_mg_per_L": level,
            "steady_state_analytic_nmol_per_L": limits,
            "steady_state_simulated_nmol_per_L": {n: float(steady[n][-1]) for n in limits}}


def regimens():
    window = (288.0, 336.0)
    times = np.round(np.arange(0, window[1] + 0.0001, 0.05), 10)
    inside = times >= window[0]
    direct, indirect = model("direct-emax.qsp"), model("indirect-response.qsp")
    summary = {}
    for label, amount, interval in (("300 mg q24h", 300, 24), ("150 mg q12h", 150, 12),
                                    ("600 mg q48h", 600, 48)):
        count = int(window[1] / interval)
        doses = schedule(amount, interval, count)
        d = simulate(direct, times, oral_doses(amount, interval, count))
        r = simulate(indirect, times, oral_doses(amount, interval, count))
        ref = idr_reference(times, doses, window[1])
        compare(f"regimen {label}", "cp", d["cp"], cp_analytic(times, doses))
        compare(f"regimen {label}", "effect", d["effect"], emax(cp_analytic(times, doses)))
        for name in ("r_kin", "r_kout"):
            compare(f"regimen {label}", name, r[name], ref[name])
        t = times[inside]
        def stats(values, scale=1.0):
            v = values[inside] * scale
            return {"min": float(v.min()), "max": float(v.max()),
                    "mean": float(np.sum((v[1:] + v[:-1]) * np.diff(t)) / 2 / (t[-1] - t[0]))}
        previous = (times >= window[0] - 48) & (times <= window[1] - 48)
        drift = {name: float(np.max(np.abs(series[inside] - series[previous])))
                 for name, series in (("cp", d["cp"]), ("r_kin", r["r_kin"]),
                                      ("r_kout", r["r_kout"]))}
        summary[label] = {
            "max_change_from_previous_48_h": drift,
            "cp_mg_per_L": stats(d["cp"]),
            "direct_effect_percent": stats(d["effect"]),
            "r_kin_percent_of_r0": stats(r["r_kin"], 100 / R0),
            "r_kout_percent_of_r0": stats(r["r_kout"], 100 / R0),
        }
    css = F * 300 / (CL * 24)
    return {"window_h": window, "average_cp_analytic_mg_per_L": css, "regimens": summary}


def emax_fit(seed=20261106):
    compiled = model("direct-emax.qsp")
    sample_times = [1, 2, 4, 6, 9, 12, 23.5, 145, 146, 148, 150, 153, 156, 167.5]
    doses = schedule(300, 24, 7)
    rng = random.Random(seed)
    sd = 3.0
    observed = [float(v) + rng.gauss(0, sd) for v in emax(cp_analytic(sample_times, doses))]
    observations = [{"row": i, "output": "effect", "value": qq.quantity(y, "%")}
                    for i, y in enumerate(observed)]
    run = qq.run_request(end=168, times=sample_times, administrations=oral_doses(300, 24, 7),
                         rtol=1e-10, atol=1e-12)
    parameters = {"emax": {"initial": 50, "lower": 10, "upper": 100, "unit": "%"},
                  "ec50": {"initial": 1.0, "lower": 0.2, "upper": 10, "unit": "mg/L"}}
    fit = compiled.fit_data(run, observations, parameters, {"model": "normal", "additive": sd})
    estimates = {n: fit["parameters"][n]["value"] for n in parameters}
    CHECKS.append({"check": "fit_data status is converged", "max_scaled_error": 0.0,
                   "passed": fit["status"] == "converged"})
    return {"seed": seed, "observations": len(observations), "additive_sd_percent": sd,
            "true": {"emax": EMAX, "ec50": EC50}, "initial": {"emax": 50, "ec50": 1.0},
            "estimates": estimates, "status": fit["status"], "simulations": fit["simulations"],
            "negative_log_likelihood": fit["objective"],
            "data": [{"time_h": t, "effect_percent": y} for t, y in zip(sample_times, observed)]}


def rounded(value, digits=4):
    if isinstance(value, float):
        return float(f"{value:.{digits}g}")
    if isinstance(value, dict):
        return {k: rounded(v, digits) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [rounded(v, digits) for v in value]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="write the unrounded summary as JSON")
    args = parser.parse_args()
    summary = {
        "direct_emax": direct_emax(),
        "effect_compartment": effect_compartment(),
        "indirect_response": indirect_response(),
        "regimen_comparison": regimens(),
        "emax_fit": emax_fit(),
    }
    summary["verification"] = {"passed": all(c["passed"] for c in CHECKS),
                               "checks": len(CHECKS),
                               "worst_scaled_error": max(c["max_scaled_error"] for c in CHECKS)}
    if args.output:
        args.output.write_text(json.dumps({**summary, "checks": CHECKS}, indent=2) + "\n")
    for check in CHECKS:
        print(f"{'pass' if check['passed'] else 'FAIL'}  {check['max_scaled_error']:.3g}  {check['check']}")
    print(json.dumps(rounded(summary), indent=2))
    if not summary["verification"]["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
