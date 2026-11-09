"""Multiple-dose pharmacokinetics of a synthetic drug given as 300 mg per day.

Three dosing schedules are compared: accumulation, steady state, an IV loading
dose and between-subject variability. Simulated pre-dose and steady-state values
are compared with analytical solutions obtained by superposition. Outputs are
written to ``--out`` (default ``multiple-dose-output``).
"""

import argparse
import copy
import csv
import json
import math
from pathlib import Path

import qspquest as qq
from single_dose_pk import CL, F, KA, K, MODEL, V

TOLERANCE = 1e-9  # mg/L
DAYS = 10
END = 24.0 * DAYS
# Dose times (h after the first dose of each day) and amounts (mg).
SCHEDULES = {
    "300 mg once daily": [(0.0, 300.0)],
    "150 mg every 12 h": [(0.0, 150.0), (12.0, 150.0)],
    "150 mg at 0 and 9 h": [(0.0, 150.0), (9.0, 150.0)],
}


def oral_term(dose, elapsed, n=None, period=24.0):
    """Concentration `elapsed` h after the last of n doses given every `period` h.

    n=None gives the steady-state value (infinitely many previous doses).
    """
    scale = F * dose * KA / (V * (KA - K))

    def geometric(rate):
        first = math.exp(-rate * elapsed)
        if n is None:
            return first / (1 - math.exp(-rate * period))
        return first * (1 - math.exp(-n * rate * period)) / (1 - math.exp(-rate * period))
    return scale * (geometric(K) - geometric(KA))


def steady_state_predose(schedule):
    """Steady-state concentration immediately before the first dose of a day."""
    return sum(oral_term(dose, 24.0 - offset) for offset, dose in schedule)


def daily_doses(schedule, days=DAYS):
    doses = []
    for offset, dose in schedule:
        administration = qq.bolus("depot", dose, "mg", at=offset, bioavailability=F)
        administration["repeat"] = {"interval": qq.quantity(24, "h"), "additional": days - 1}
        doses.append(administration)
    return doses


def with_observations(request, observations):
    request = copy.deepcopy(request)
    request["regimen"]["observations"] = [
        {"time": qq.quantity(t, "h"), "side": side} for t, side in observations]
    return request


def cp(result):
    return next(c for c in result["outputs"] if c["name"] == "cp")["values"]


def main(out):
    out.mkdir(parents=True, exist_ok=False)
    model = qq.CompiledModel(MODEL)
    rows = []

    # Steady state over one 24 h dosing cycle.
    steady = {}
    for label, schedule in SCHEDULES.items():
        cycle = qq.run_request(end=24, times=[24], solver="analytic_linear_pk",
                               administrations=daily_doses(schedule, days=1))
        del cycle["regimen"]["observations"]  # steady-state requests take no samples
        result = model.periodic_steady_state({
            "run": cycle, "absolute_tolerance": qq.quantity(1e-9, "mg"),
            "relative_tolerance": 1e-9})
        predose = result["initial_states"]["central"]["value"] / V
        assert abs(predose - steady_state_predose(schedule)) < TOLERANCE
        steady[label] = {"cycle": cycle, "result": result, "predose": predose}

    # Accumulation over 10 days, sampled immediately before each day's first dose.
    print(f"Half-life {math.log(2) / K:.2f} h")
    print("Pre-dose concentration as a percentage of steady state")
    print("Schedule               Day 1   Day 2   Day 3   Day 4   Day 10")
    for label, schedule in SCHEDULES.items():
        run = qq.run_request(end=END, times=[24], solver="analytic_linear_pk",
                             administrations=daily_doses(schedule))
        days = range(1, DAYS + 1)
        values = cp(model.run(with_observations(run, [(24.0 * d, "pre") for d in days])))
        for day, value in zip(days, values):
            exact = sum(oral_term(dose, 24.0 - offset, n=day) for offset, dose in schedule)
            assert abs(value - exact) < TOLERANCE, (label, day, value, exact)
            rows.append({"schedule": label, "day": day, "time_h": 24.0 * day,
                         "predose_mg_per_L": value,
                         "percent_of_steady_state": 100 * value / steady[label]["predose"]})
        percent = [100 * v / steady[label]["predose"] for v in values]
        print(f"{label:20s}" + "".join(f"{p:8.1f}" for p in percent[:4])
              + f"{percent[-1]:9.3f}")

    print("\nSteady state (one 24 h cycle)")
    print("Schedule              AUC0-24 (mg*h/L)  Cmax (mg/L)  Cmin (mg/L)  "
          "Cmax/Cmin  Residual")
    profiles = {}
    for label, record in steady.items():
        day = copy.deepcopy(record["cycle"])
        day["initial_states"] = record["result"]["initial_states"]
        grid = [(i * 0.05, "post") for i in range(481)]
        s = qq.exposure_summary(model.run(with_observations(day, grid)), "cp")
        assert abs(s["auc"] - F * 300 / CL) / (F * 300 / CL) < 1e-3
        profiles[label] = s
        print(f"{label:20s} {s['auc']:17.2f} {s['cmax']:12.3f} {s['cmin']:12.3f} "
              f"{s['cmax'] / s['cmin']:10.2f} {record['result']['maximum_scaled_residual']:9.1e}")

    # IV loading dose from the accumulation ratio; pre- and post-dose samples.
    dose, tau = 150.0, 12.0
    ratio = 1 / (1 - math.exp(-K * tau))
    loading = dose * ratio
    maintenance = qq.bolus("central", dose, "mg", at=tau)
    maintenance["repeat"] = {"interval": qq.quantity(tau, "h"), "additional": 8}
    loaded = qq.run_request(end=120, times=[12], solver="analytic_linear_pk",
                            administrations=[qq.bolus("central", loading, "mg"), maintenance])
    samples = [(tau * i, side) for i in range(1, 10) for side in ("pre", "post")]
    values = cp(model.run(with_observations(loaded, samples)))
    pre, post = values[0::2], values[1::2]
    ss_trough = dose / V * math.exp(-K * tau) * ratio
    assert all(abs(c - ss_trough) < TOLERANCE for c in pre)
    assert all(abs(b - a - dose / V) < TOLERANCE for a, b in zip(pre, post))
    unloaded = qq.bolus("central", dose, "mg")
    unloaded["repeat"] = {"interval": qq.quantity(tau, "h"), "additional": 9}
    plain = cp(model.run(with_observations(
        qq.run_request(end=120, times=[12], solver="analytic_linear_pk",
                       administrations=[unloaded]), samples[0::2])))
    print(f"\nIV 150 mg every 12 h: accumulation ratio {ratio:.4f}, loading dose "
          f"{loading:.1f} mg")
    print(f"With loading dose: Ctrough {pre[0]:.4f} mg/L and Cmax {post[0]:.4f} mg/L "
          "at every dose")
    print(f"Without loading dose: Ctrough {plain[0]:.4f} mg/L (dose 2) to "
          f"{plain[-1]:.4f} mg/L (dose 10)")

    # Between-subject variability; every virtual subject receives every schedule.
    final_day = [216 + 0.25 * i for i in range(97)]
    regimens = {label: qq.run_request(end=END, times=final_day, solver="analytic_linear_pk",
                                      administrations=daily_doses(schedule))
                for label, schedule in SCHEDULES.items()}
    comparison = model.compare_regimens(
        regimens,
        {"cl": qq.quantity(CL, "L/h"), "v": qq.quantity(V, "L"), "ka": qq.quantity(KA, "1/h")},
        [[0.09, 0.0], [0.0, 0.04]], "cp", random_effects=["cl", "v"],
        subjects=400, seed=20261105)
    print("\nDay 10, 400 virtual subjects: median (5th-95th percentile)")
    print("Schedule              Cmax (mg/L)          Cmin (mg/L)          "
          "Change in Cmax vs once daily")
    reference = comparison["reference"]
    for label in regimens:  # the declared order; result keys are not ordered
        record = comparison["regimens"][label]
        m = record["metric_percentiles"]
        line = f"{label:20s} " + " ".join(
            f"{m[key][1]:.2f} ({m[key][0]:.2f}-{m[key][2]:.2f})".ljust(20)
            for key in ("cmax", "cmin"))
        if label != reference:
            d = record["paired_differences_vs_reference"]["cmax"]
            worst = max(a["cmax"] - b["cmax"] for a, b in zip(
                record["subject_metrics"], comparison["regimens"][reference]["subject_metrics"]))
            assert worst < 0
            line += f" {d[1]:.2f} ({d[0]:.2f} to {d[2]:.2f})"
        print(line.rstrip())

    with (out / "predose.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    (out / "model.qsp").write_text(MODEL)
    (out / "steady-state.json").write_text(json.dumps(
        {k: v["result"] for k, v in steady.items()}, indent=2))
    (out / "regimen-comparison.json").write_text(json.dumps(comparison, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("multiple-dose-output"))
    main(parser.parse_args().out)
