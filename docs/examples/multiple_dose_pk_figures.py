"""Figures for the multiple-dose pharmacokinetics tutorial.

Simulates the schedules from multiple_dose_pk.py, compares the simulated values
with the superposition solutions, and writes four PNG figures to
docs/tutorials/figures/. Requires the ``docs`` extra (pandas and plotnine).
"""

import copy

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, element_text, facet_wrap, geom_abline, geom_hline, geom_line, geom_point,
    geom_raster, geom_vline, ggplot, labs, scale_color_manual, scale_fill_gradientn,
    scale_x_continuous, theme,
)

import qspquest as qq
from figure_style import CATEGORICAL, INK, SEQUENTIAL, save, style
from multiple_dose_pk import (
    CL, END, F, KA, SCHEDULES, TOLERANCE, V, cp, daily_doses, oral_term, with_observations,
)
from single_dose_pk import MODEL

ORDER = list(SCHEDULES)


def steady_state(model, schedule):
    cycle = qq.run_request(end=24, times=[24], solver="analytic_linear_pk",
                           administrations=daily_doses(schedule, days=1))
    del cycle["regimen"]["observations"]
    result = model.periodic_steady_state({
        "run": cycle, "absolute_tolerance": qq.quantity(1e-9, "mg"),
        "relative_tolerance": 1e-9})
    return cycle, result["initial_states"]


def steady_day(model, schedule, step=0.1):
    """Concentrations over one steady-state day, checked against superposition."""
    cycle, states = steady_state(model, schedule)
    day = copy.deepcopy(cycle)
    day["initial_states"] = states
    times = [round(i * step, 10) for i in range(int(24 / step) + 1)]
    values = np.asarray(cp(model.run(with_observations(day, [(t, "post") for t in times]))))
    exact = np.asarray([steady_value(schedule, t) for t in times])
    assert np.max(np.abs(values - exact)) < TOLERANCE
    return np.asarray(times), values


def steady_value(schedule, t):
    """Steady-state concentration at time t of the dosing day, by superposition."""
    return sum(oral_term(dose, t - offset if t >= offset else t - offset + 24.0)
               for offset, dose in schedule)


def accumulation(model):
    """Figure 1: concentrations over 10 days with steady-state limits."""
    times = [round(i * 0.25, 10) for i in range(int(END / 0.25) + 1)]
    lines, limits = [], []
    for label, schedule in SCHEDULES.items():
        run = qq.run_request(end=END, times=[24], solver="analytic_linear_pk",
                             administrations=daily_doses(schedule))
        values = cp(model.run(with_observations(run, [(t, "post") for t in times])))
        lines.append(pd.DataFrame({"time": times, "conc": values, "schedule": label}))
        _, ss = steady_day(model, schedule)
        limits.append(pd.DataFrame({"y": [ss.min(), ss.max()], "schedule": label,
                                    "kind": ["Cmin,ss", "Cmax,ss"]}))
    lines, limits = pd.concat(lines), pd.concat(limits)
    for frame in (lines, limits):
        frame["schedule"] = pd.Categorical(frame["schedule"], ORDER)
    plot = (ggplot()
            + geom_hline(limits, aes(yintercept="y"), color=INK, linetype="dashed", size=0.45)
            + geom_line(lines, aes("time", "conc", color="schedule"), size=0.6)
            + facet_wrap("~schedule", ncol=1)
            + scale_color_manual(values=CATEGORICAL)
            + scale_x_continuous(breaks=list(range(0, 241, 24)),
                                 labels=[str(d) for d in range(0, 11)])
            + labs(x="Time (days)", y="Plasma concentration (mg/L)")
            + style(width=7.0, height=5.6) + theme(legend_position="none"))
    return save(plot, "multiple-dose-accumulation")


def steady_profiles(model):
    """Figure 2: one steady-state day for each schedule."""
    rows, doses = [], []
    for label, schedule in SCHEDULES.items():
        t, c = steady_day(model, schedule)
        rows.append(pd.DataFrame({"time": t, "conc": c, "schedule": label}))
        doses.extend({"time": offset, "schedule": label} for offset, _ in schedule)
    rows, doses = pd.concat(rows), pd.DataFrame(doses)
    for frame in (rows, doses):
        frame["schedule"] = pd.Categorical(frame["schedule"], ORDER)
    auc = F * 300 / CL / 24
    plot = (ggplot()
            + geom_hline(yintercept=auc, color=INK, linetype="dotted", size=0.5)
            + annotate("text", x=24, y=auc + 0.03, label="Average Css", ha="right",
                       va="bottom", color=INK, size=8.5)
            + geom_line(rows, aes("time", "conc", color="schedule"), size=0.9)
            + geom_point(doses.merge(rows, on=["time", "schedule"]),
                         aes("time", "conc", color="schedule"), size=2.6, shape="v")
            + scale_color_manual(values=CATEGORICAL)
            + scale_x_continuous(breaks=[0, 3, 6, 9, 12, 15, 18, 21, 24])
            + labs(x="Time of day (h after the first dose)",
                   y="Steady-state concentration (mg/L)")
            + style(width=7.0, height=4.0))
    return save(plot, "multiple-dose-steady-state")


def design_map(model):
    """Figure 3: steady-state Cmin over the time and size of the second daily dose."""
    offsets = list(range(1, 24))
    fractions = [round(0.2 + 0.05 * i, 2) for i in range(13)]
    rows = []
    for offset in offsets:
        for first in fractions:
            schedule = [(0.0, 300.0 * first), (float(offset), 300.0 * (1 - first))]
            cycle, states = steady_state(model, schedule)
            predose = states["central"]["value"] / V
            assert abs(predose - sum(oral_term(d, 24.0 - o) for o, d in schedule)) < TOLERANCE
            day = copy.deepcopy(cycle)
            day["initial_states"] = states
            grid = [(round(i * 0.25, 10), "post") for i in range(97)]
            values = cp(model.run(with_observations(day, grid)))
            rows.append({"offset": offset, "first": first, "cmin": min(values),
                         "cmax": max(values)})
    grid = pd.DataFrame(rows)
    plot = (ggplot(grid, aes("offset", "first"))
            + geom_raster(aes(fill="cmin"))
            + geom_vline(xintercept=12, color="white", linetype="dashed", size=0.5)
            + annotate("point", x=12, y=0.5, color="white", size=3)
            + annotate("point", x=9, y=0.5, color="white", size=3, shape="s")
            + annotate("text", x=12.4, y=0.52, label="150 mg every 12 h", ha="left",
                       color="white", size=8.5)
            + annotate("text", x=8.6, y=0.47, label="0 and 9 h", ha="right",
                       color="white", size=8.5)
            + scale_fill_gradientn(colors=SEQUENTIAL, name="Cmin,ss (mg/L)")
            + scale_x_continuous(breaks=[1, 3, 6, 9, 12, 15, 18, 21, 23], expand=(0, 0))
            + labs(x="Time of the second daily dose (h after the first)",
                   y="Fraction of 300 mg in the first dose")
            + style(width=7.0, height=4.6)
            + theme(legend_position="right", legend_title=element_text(color=INK, size=9)))
    best = grid.loc[grid["cmin"].idxmax()]
    return save(plot, "multiple-dose-design-map"), best


def paired_subjects(model):
    """Figure 4: day-10 Cmax and Cmin in the same virtual subjects."""
    final_day = [216 + 0.25 * i for i in range(97)]
    regimens = {label: qq.run_request(end=END, times=final_day, solver="analytic_linear_pk",
                                      administrations=daily_doses(schedule))
                for label, schedule in SCHEDULES.items()}
    comparison = model.compare_regimens(
        regimens,
        {"cl": qq.quantity(CL, "L/h"), "v": qq.quantity(V, "L"), "ka": qq.quantity(KA, "1/h")},
        [[0.09, 0.0], [0.0, 0.04]], "cp", random_effects=["cl", "v"],
        subjects=400, seed=20261105)
    reference = comparison["reference"]
    base = comparison["regimens"][reference]["subject_metrics"]
    rows = []
    for label in ORDER[1:]:
        for ref, other in zip(base, comparison["regimens"][label]["subject_metrics"]):
            for metric, name in (("cmax", "A  Cmax on day 10"), ("cmin", "B  Cmin on day 10")):
                rows.append({"once": ref[metric], "other": other[metric], "metric": name,
                             "schedule": label})
    points = pd.DataFrame(rows)
    points["schedule"] = pd.Categorical(points["schedule"], ORDER[1:])
    cmax = points[points["metric"].str.contains("Cmax")]
    assert (cmax["other"] < cmax["once"]).all()
    plot = (ggplot(points, aes("once", "other", color="schedule"))
            + geom_abline(intercept=0, slope=1, color=INK, linetype="dashed", size=0.5)
            + geom_point(size=1.3, alpha=0.55, stroke=0)
            + facet_wrap("~metric", scales="free")
            + scale_color_manual(values=CATEGORICAL[1:])
            + labs(x="300 mg once daily (mg/L)", y="Divided daily dose (mg/L)")
            + style(width=7.4, height=4.0))
    return save(plot, "multiple-dose-paired-subjects")


def main():
    model = qq.CompiledModel(MODEL)
    print(accumulation(model).name)
    print(steady_profiles(model).name)
    path, best = design_map(model)
    print(path.name, f"(highest Cmin,ss {best['cmin']:.3f} mg/L at {best['offset']:g} h, "
                     f"first-dose fraction {best['first']:g})")
    print(paired_subjects(model).name)


if __name__ == "__main__":
    main()
