"""Figures for the noncompartmental analysis tutorial.

Repeats the simulations from noncompartmental_analysis.py, checks each plotted
value against the NCA result or the closed-form solution, and writes three PNG
figures to docs/tutorials/figures/. Requires the ``docs`` extra.
"""

import math

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, element_text, facet_wrap, geom_hline, geom_line, geom_point,
    geom_ribbon, geom_segment, ggplot, labs, position_dodge, scale_color_manual,
    scale_fill_manual, scale_linetype_manual, scale_shape_manual, scale_x_continuous,
    scale_x_log10, scale_y_continuous, scale_y_log10, theme,
)
from plotnine.composition import Beside

import qspquest as qq
from figure_style import CATEGORICAL, FILL_LIGHT, INK, MUTED, save, style
from noncompartmental_analysis import (
    BLQ_RULE, CL, DOSE, F, K, KA, LLOQ, SOURCE, TIMES, UNITS, V,
    oral_closed_form, profile_rows, simulate, simulated_subjects,
)

ORAL = [{"subject": "oral", "amount": DOSE, "route": "extravascular"}]
FLIP_FLOP_TIMES = TIMES + [96, 120, 144, 168]


def values(profile):
    return {code: item["value"] for code, item in profile["parameters"].items()}


def terminal_phase(model):
    """Figure 1: λz regression for each simulated subject."""
    observations, doses, _ = simulated_subjects(model)
    result = qq.noncompartmental_analysis(observations, doses, units=UNITS, blq=BLQ_RULE)
    points, lines, labels = [], [], {}
    for profile in result["profiles"]:
        p, terminal = values(profile), profile["lambda_z"]
        subject = profile["subject"]
        used = [r for r in profile["rows"] if r["role"] == "auc_and_lambda_z"]
        assert [r["time"] for r in used] == terminal["times"]
        assert len(used) == p["LAMZNPT"] and used[0]["time"] == p["LAMZLL"]
        x = np.array([r["time"] for r in used])
        y = np.log([r["concentration"] for r in used])
        slope, intercept = np.polyfit(x, y, 1)
        assert math.isclose(-slope, p["LAMZ"], rel_tol=1e-9)
        assert math.isclose(intercept, terminal["intercept"], rel_tol=1e-9)
        labels[subject] = f"Subject {subject}\nt½ {p['LAMZHL']:.1f} h, {p['LAMZNPT']} points"
        for row in profile["rows"]:
            if row["role"] == "predose":
                continue
            kind = {"auc_and_lambda_z": "λz regression", "auc": "AUC only",
                    "trailing_blq_excluded": "BLQ, excluded"}[row["role"]]
            conc = LLOQ if row["blq"] else row["concentration"]
            points.append({"subject": subject, "time": row["time"], "conc": conc, "kind": kind})
        span = np.linspace(p["LAMZLL"], p["TLST"], 50)
        lines.append(pd.DataFrame({"subject": subject, "time": span,
                                   "conc": np.exp(terminal["intercept"] - p["LAMZ"] * span)}))
        print(f"Subject {subject}: λz {p['LAMZ']:.4f} 1/h from {p['LAMZNPT']} points "
              f"({p['LAMZLL']:g}-{p['LAMZUL']:g} h)")
    points, lines = pd.DataFrame(points), pd.concat(lines)
    order = [labels[s] for s in sorted(labels)]
    for frame in (points, lines):
        frame["panel"] = pd.Categorical(frame["subject"].map(labels), order)
    points["kind"] = pd.Categorical(points["kind"], ["λz regression", "AUC only", "BLQ, excluded"])
    plot = (ggplot()
            + geom_hline(yintercept=LLOQ, color=MUTED, linetype="dashed", size=0.4)
            + geom_line(lines, aes("time", "conc"), color=INK, size=0.7)
            + geom_point(points, aes("time", "conc", color="kind", shape="kind", fill="kind"),
                         size=2.0, stroke=0.8)
            + facet_wrap("~panel", ncol=4)
            + scale_y_log10(breaks=[0.1, 0.3, 1, 3], labels=["0.1", "0.3", "1", "3"])
            + scale_x_continuous(breaks=[0, 24, 48, 72, 96])
            + scale_color_manual(values=[CATEGORICAL[0], INK, CATEGORICAL[1]])
            + scale_fill_manual(values=[CATEGORICAL[0], "white", CATEGORICAL[1]])
            + scale_shape_manual(values=["o", "o", "X"])
            + labs(x="Time after dose (h)", y="Plasma concentration (mg/L, log scale)")
            + style(width=7.4, height=4.8)
            + theme(strip_text=element_text(color=INK, size=8.5, ha="left")))
    return save(plot, "nca-terminal-phase")


def interval_area(t1, t2):
    """Closed-form area of the oral profile between t1 and t2 (mg·h/L)."""
    scale = F * DOSE * KA / (V * (KA - K))
    return scale * ((math.exp(-K * t1) - math.exp(-K * t2)) / K
                    - (math.exp(-KA * t1) - math.exp(-KA * t2)) / KA)


def auc_rules(model):
    """Figure 2: interval areas from the two trapezoidal rules."""
    conc = simulate(model, "depot", F, TIMES)
    rows = profile_rows("oral", TIMES, conc)
    t, c = [0.0] + TIMES, [0.0] + list(conc)
    areas = {"Linear": [], "Linear-up/log-down": []}
    for t1, t2, c1, c2 in zip(t, t[1:], c, c[1:]):
        dt = t2 - t1
        linear = dt * (c1 + c2) / 2
        areas["Linear"].append(linear)
        down = c1 > c2 > 0
        areas["Linear-up/log-down"].append(dt * (c1 - c2) / math.log(c1 / c2) if down else linear)
    for method, rule in (("Linear", "linear"), ("Linear-up/log-down", "linear_up_log_down")):
        p = values(qq.noncompartmental_analysis(rows, ORAL, units=UNITS, auc_method=rule)
                   ["profiles"][0])
        assert math.isclose(sum(areas[method]), p["AUCLST"], rel_tol=1e-12)
    exact = [interval_area(t1, t2) for t1, t2 in zip(t, t[1:])]
    names = [f"{t1:g}–{t2:g}" for t1, t2 in zip(t, t[1:])]
    errors = pd.DataFrame([
        {"interval": name, "method": method, "error": 100 * (area / ref - 1)}
        for method, values_ in areas.items()
        for name, area, ref in zip(names, values_, exact)
    ])
    errors["interval"] = pd.Categorical(errors["interval"], names)
    for name in names[-4:]:
        row = errors[errors["interval"] == name].set_index("method")["error"]
        print(f"Interval {name} h: linear {row['Linear']:+.2f}%, "
              f"linear-up/log-down {row['Linear-up/log-down']:+.4f}%")

    dense = np.linspace(24, 72, 481)
    curves = [pd.DataFrame({"time": dense, "conc": [oral_closed_form(x) for x in dense],
                            "kind": "Analytical"})]
    linear = np.interp(dense, t, c)
    index = np.searchsorted(t, dense, side="right").clip(1, len(t) - 1) - 1
    t1, t2 = np.array(t)[index], np.array(t)[index + 1]
    c1, c2 = np.array(c)[index], np.array(c)[index + 1]
    logdown = c1 * np.exp(np.log(c2 / c1) * (dense - t1) / (t2 - t1))
    curves.append(pd.DataFrame({"time": dense, "conc": linear, "kind": "Linear"}))
    curves.append(pd.DataFrame({"time": dense, "conc": logdown, "kind": "Linear-up/log-down"}))
    curves = pd.concat(curves)
    curves["kind"] = pd.Categorical(curves["kind"], ["Analytical", "Linear", "Linear-up/log-down"])
    shade = pd.DataFrame({"time": dense, "low": [oral_closed_form(x) for x in dense],
                          "high": linear})
    samples = pd.DataFrame({"time": TIMES, "conc": conc})
    samples = samples[samples["time"] >= 24]
    colors = [INK, CATEGORICAL[1], CATEGORICAL[0]]
    profile = (ggplot()
               + geom_ribbon(shade, aes("time", ymin="low", ymax="high"), fill=FILL_LIGHT,
                             alpha=0.6)
               + geom_line(curves, aes("time", "conc", color="kind", linetype="kind"), size=0.7)
               + geom_point(samples, aes("time", "conc"), color=INK, size=2.0)
               + scale_color_manual(values=colors)
               + scale_linetype_manual(values=["solid", "dashed", "dotted"])
               + scale_x_continuous(breaks=[24, 36, 48, 60, 72])
               + labs(x="Time after dose (h)", y="Plasma concentration (mg/L)",
                      title="A  Interpolation between samples, 24–72 h")
               + style(width=3.9, height=4.4))
    dodge = position_dodge(width=0.6)
    bars = (ggplot(errors, aes("interval", "error", color="method"))
            + geom_hline(yintercept=0, color=MUTED, size=0.4)
            + geom_segment(aes(xend="interval", y=0, yend="error"), position=dodge, size=0.7)
            + geom_point(position=dodge, size=2.0)
            + scale_color_manual(values=colors[1:])
            + scale_y_continuous(breaks=[-4, -2, 0, 2, 4, 6, 8])
            + labs(x="Sampling interval (h)", y="Interval AUC error vs analytical (%)",
                   title="B  Error of each interval area")
            + style(width=4.6, height=4.4)
            + theme(axis_text_x=element_text(rotation=90, size=8), legend_position="none"))
    plot = Beside([profile, bars]) + theme(figure_size=(8.5, 4.6))
    return save(plot, "nca-auc-rules")


def flip_flop(model):
    """Figure 3: NCA λz, CL/F and Vz/F across absorption rate constants."""
    rows = []
    for ka in np.geomspace(0.005, 2.0, 25):
        conc = simulate(model, "depot", F, FLIP_FLOP_TIMES,
                        {"ka": {"value": float(ka), "unit": "1/h"}})
        profile = qq.noncompartmental_analysis(
            profile_rows("oral", FLIP_FLOP_TIMES, conc), ORAL, units=UNITS)["profiles"][0]
        assert profile["lambda_z"]["status"] == "estimated", ka
        p = values(profile)
        rows.append({"ka": ka, "lz": p["LAMZ"], "points": p["LAMZNPT"],
                     "span": p["LAMZSPN"], "extrap": p["AUCPEO"], "clf": p["CLFO"],
                     "vzf": p["VZFO"]})
    sweep = pd.DataFrame(rows)
    for r in sweep.itertuples():
        print(f"KA {r.ka:.4f}: λz {r.lz:.5f} 1/h, {r.points} points, span {r.span:.2f} t½, "
              f"AUC%extrap {r.extrap:.1f}, CL/F {r.clf:.3f} L/h, Vz/F {r.vzf:.1f} L")
    limit = np.minimum(sweep["ka"], K)
    assert (abs(sweep["lz"] / limit - 1) < 0.2).all()
    assert (sweep["lz"] <= limit * (1 + 1e-9)).all()
    assert abs(sweep["lz"].iloc[0] / sweep["ka"].iloc[0] - 1) < 0.06
    assert abs(sweep["lz"].iloc[-1] / K - 1) < 0.001

    reference = pd.DataFrame({"ka": np.geomspace(0.005, 2.0, 200)})
    reference["lambda"] = np.minimum(reference["ka"], K)
    rates = (ggplot()
             + annotate("rect", xmin=0.005, xmax=K, ymin=0.004, ymax=0.06, fill=FILL_LIGHT,
                        alpha=0.25)
             + geom_line(reference, aes("ka", "lambda"), color=INK, linetype="dashed", size=0.6)
             + geom_point(sweep, aes("ka", "lz"), color=CATEGORICAL[1], size=2.2)
             + annotate("text", x=0.0055, y=0.052, label="KA < k: flip-flop", ha="left",
                        color=INK, size=8.5)
             + annotate("text", x=1.9, y=K * 1.1, label="k = 0.04 h⁻¹", ha="right",
                        va="bottom", color=INK, size=8.5)
             + scale_x_log10(breaks=[0.005, 0.01, 0.04, 0.1, 0.4, 2],
                             labels=["0.005", "0.01", "0.04", "0.1", "0.4", "2"])
             + scale_y_log10(breaks=[0.005, 0.01, 0.02, 0.04], limits=(0.004, 0.06),
                             labels=["0.005", "0.01", "0.02", "0.04"])
             + labs(x="Absorption rate constant KA (h⁻¹, log scale)",
                    y="NCA λz (h⁻¹, log scale)", title="A  Terminal rate constant")
             + style(width=4.2, height=4.2))
    ratios = pd.concat([
        pd.DataFrame({"ka": sweep["ka"], "ratio": sweep["clf"] / (CL / F), "kind": "CL/F"}),
        pd.DataFrame({"ka": sweep["ka"], "ratio": sweep["vzf"] / (V / F), "kind": "Vz/F"}),
    ])
    ratios["kind"] = pd.Categorical(ratios["kind"], ["CL/F", "Vz/F"])
    apparent = (ggplot(ratios, aes("ka", "ratio", color="kind"))
                + annotate("rect", xmin=0.005, xmax=K, ymin=0.8, ymax=10, fill=FILL_LIGHT,
                           alpha=0.25)
                + geom_hline(yintercept=1, color=INK, linetype="dashed", size=0.6)
                + geom_line(size=0.6)
                + geom_point(size=2.0)
                + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[2]])
                + scale_x_log10(breaks=[0.005, 0.01, 0.04, 0.1, 0.4, 2],
                                labels=["0.005", "0.01", "0.04", "0.1", "0.4", "2"])
                + scale_y_log10(breaks=[1, 2, 4, 8], limits=(0.8, 10))
                + labs(x="Absorption rate constant KA (h⁻¹, log scale)",
                       y="NCA estimate / simulation input (log scale)",
                       title="B  Apparent clearance and volume")
                + style(width=4.2, height=4.2))
    plot = Beside([rates, apparent]) + theme(figure_size=(8.5, 4.4))
    return save(plot, "nca-flip-flop")


def main():
    model = qq.CompiledModel(SOURCE)
    for figure in (terminal_phase, auc_rules, flip_flop):
        print(figure(model).name)


if __name__ == "__main__":
    main()
