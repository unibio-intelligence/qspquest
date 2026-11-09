"""Figures for the single-dose pharmacokinetics tutorial.

Simulates the model from single_dose_pk.py, compares every simulated value with
the analytical solution, and writes four PNG figures to docs/tutorials/figures/.
Requires the ``docs`` extra (pandas and plotnine).
"""

import math

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, facet_wrap, geom_area, geom_hline, geom_line, geom_path, geom_point,
    element_text, geom_raster, ggplot, labs, scale_color_manual, scale_fill_gradientn, scale_x_continuous,
    scale_x_log10, scale_y_continuous, scale_y_log10, theme,
)

import qspquest as qq
from plotnine.composition import Beside
from figure_style import CATEGORICAL, FILL_LIGHT, INK, SEQUENTIAL, save, style
from single_dose_pk import (
    K, MODEL, ROUTES, SPARSE, TIMES, TOLERANCE, cp, oral_auc, oral_cp, request,
)

ROUTE_ORDER = list(ROUTES)


def simulated(model, administration, times, parameters=None, end=72):
    result = model.run(request(administration, times, parameters=parameters, end=end))
    return np.asarray(result["times"]), np.asarray(cp(result))


def profiles(model):
    """Figure 1: concentration-time profiles for three routes."""
    dense = np.linspace(0, 72, 721)
    lines, points = [], []
    for route, (dose, exact) in ROUTES.items():
        t, c = simulated(model, dose(), TIMES)
        assert max(abs(c - [exact(x) for x in t])) < TOLERANCE
        lines.append(pd.DataFrame({"time": dense, "conc": [exact(x) for x in dense],
                                   "route": route}))
        points.append(pd.DataFrame({"time": t, "conc": c, "route": route}))
    lines, points = pd.concat(lines), pd.concat(points)
    for frame in (lines, points):
        frame["route"] = pd.Categorical(frame["route"], ROUTE_ORDER)
    positive = points[points["conc"] > 0]
    early_lines, early_points = lines[lines["time"] <= 6], points[points["time"] <= 6]
    early = (ggplot()
             + geom_line(early_lines, aes("time", "conc", color="route"), size=0.8)
             + geom_point(early_points, aes("time", "conc", color="route"), size=2.2,
                          fill="white", stroke=0.9, shape="o")
             + scale_x_continuous(breaks=[0, 1, 2, 3, 4, 5, 6])
             + scale_color_manual(values=CATEGORICAL)
             + labs(x="Time after dose (h)", y="Plasma concentration (mg/L)",
                    title="A  0–6 h, linear scale")
             + style(width=3.6, height=4.2) + theme(legend_position="none"))
    late = (ggplot()
            + geom_line(lines[lines["conc"] > 1e-3], aes("time", "conc", color="route"), size=0.8)
            + geom_point(positive, aes("time", "conc", color="route"), size=2.2,
                         fill="white", stroke=0.9, shape="o")
            + scale_y_log10()
            + scale_x_continuous(breaks=[0, 12, 24, 36, 48, 60, 72])
            + scale_color_manual(values=CATEGORICAL)
            + labs(x="Time after dose (h)", y="Plasma concentration (mg/L, log scale)",
                   title="B  0–72 h, log scale")
            + style(width=4.4, height=4.2))
    plot = Beside([early, late]) & theme(figure_size=(8.0, 4.4))
    return save(plot, "single-dose-profiles")


def sampling(model):
    """Figure 2: trapezoidal AUC for two sampling schedules, oral dose."""
    dense = np.linspace(0, 72, 1441)
    exact = np.array([oral_cp(x) for x in dense])
    curves, samples, fills = [], [], []
    for label, times in (("8 samples", SPARSE), ("15 samples", TIMES)):
        t, c = simulated(model, ROUTES["Oral"][0](), times)
        assert max(abs(c - [oral_cp(x) for x in t])) < TOLERANCE
        auc = float(np.dot(np.diff(t), (c[:-1] + c[1:]) / 2))
        panel = f"{label}: AUC0–72 {auc:.2f} mg·h/L ({100 * (auc / oral_auc(72) - 1):+.1f}%)"
        linear = np.interp(dense, t, c)
        curves.append(pd.DataFrame({"time": dense, "conc": linear, "panel": panel,
                                    "kind": "Linear trapezoidal interpolation"}))
        curves.append(pd.DataFrame({"time": dense, "conc": exact, "panel": panel,
                                    "kind": "Analytical solution"}))
        fills.append(pd.DataFrame({"time": dense, "low": exact, "high": linear, "panel": panel}))
        samples.append(pd.DataFrame({"time": t, "conc": c, "panel": panel}))
    curves, samples, fills = pd.concat(curves), pd.concat(samples), pd.concat(fills)
    order = list(dict.fromkeys(curves["panel"]))
    for frame in (curves, samples, fills):
        frame["panel"] = pd.Categorical(frame["panel"], order)
    plot = (ggplot()
            + geom_area(fills.assign(conc=fills["high"]), aes("time", "conc"),
                        fill=FILL_LIGHT, alpha=0.35)
            + geom_line(curves, aes("time", "conc", color="kind", linetype="kind"), size=0.75)
            + geom_point(samples, aes("time", "conc"), color=INK, size=1.8)
            + facet_wrap("~panel", ncol=1)
            + scale_color_manual(values=[INK, CATEGORICAL[0]])
            + scale_x_continuous(breaks=[0, 8, 24, 48, 72])
            + labs(x="Time after dose (h)", y="Plasma concentration (mg/L)",
                   color="", linetype="")
            + style(width=7.0, height=5.2))
    return save(plot, "single-dose-sampling")


def absorption_map(model):
    """Figure 3: concentration over time and absorption rate constant."""
    rates = np.geomspace(0.005, 2.0, 41)
    times = list(range(0, 169))
    rows = []
    for ka in rates:
        t, c = simulated(model, ROUTES["Oral"][0](), times, end=168,
                         parameters={"ka": qq.quantity(float(ka), "1/h")})
        expected = [oral_cp(x, ka) for x in t]
        assert max(abs(c - expected)) < TOLERANCE, ka
        rows.append(pd.DataFrame({"time": t, "ka": ka, "conc": c}))
    grid = pd.concat(rows)
    grid["log_ka"] = np.log10(grid["ka"])
    grid["log_conc"] = np.log10(np.clip(grid["conc"], 1e-3, None))
    tmax = pd.DataFrame({"ka": rates})
    tmax["time"] = [math.log(a / K) / (a - K) if abs(a - K) > 1e-12 else 1 / K for a in rates]
    tmax["log_ka"] = np.log10(tmax["ka"])
    tmax = tmax[tmax["time"] <= 168]
    ticks = [0.005, 0.01, 0.02, 0.04, 0.1, 0.2, 0.5, 1, 2]
    plot = (ggplot(grid, aes("time", "log_ka"))
            + geom_raster(aes(fill="log_conc"), interpolate=True)
            + geom_path(tmax, aes("time", "log_ka"), color="white", size=0.9)
            + geom_hline(yintercept=math.log10(K), color="white", linetype="dashed", size=0.6)
            + annotate("text", x=165, y=math.log10(K) + 0.08, label="KA = k", ha="right",
                       color="white", size=9)
            + annotate("text", x=150, y=math.log10(0.006), label="KA < k: flip-flop",
                       ha="right", color=INK, size=9)
            + annotate("text", x=19, y=math.log10(0.12), label="Tmax", ha="left",
                       color="white", size=9)
            + scale_fill_gradientn(colors=SEQUENTIAL, name="C (mg/L)",
                                   limits=(-3, math.log10(grid["conc"].max())),
                                   breaks=[-3, -2, -1, 0], labels=["0.001", "0.01", "0.1", "1"])
            + scale_x_continuous(breaks=[0, 24, 48, 72, 96, 120, 144, 168], expand=(0, 0))
            + scale_y_continuous_log_ticks(ticks)
            + labs(x="Time after dose (h)", y="Absorption rate constant KA (h⁻¹, log scale)")
            + style(width=7.0, height=4.6)
            + theme(legend_position="right",
                    legend_title=element_text(color=INK, size=9)))
    return save(plot, "single-dose-absorption-map")


def scale_y_continuous_log_ticks(ticks):
    return scale_y_continuous(breaks=[math.log10(t) for t in ticks],
                              labels=[f"{t:g}" for t in ticks], expand=(0, 0))


def terminal_slope(model):
    """Figure 4: apparent terminal rate constant between 144 and 168 h."""
    rates = np.geomspace(0.005, 2.0, 25)
    rows = []
    for ka in rates:
        t, c = simulated(model, ROUTES["Oral"][0](), [144, 168], end=168,
                         parameters={"ka": qq.quantity(float(ka), "1/h")})
        rows.append({"ka": ka, "lambda": -math.log(c[1] / c[0]) / 24})
    points = pd.DataFrame(rows)
    reference = pd.DataFrame({"ka": np.geomspace(0.005, 2.0, 200)})
    reference["lambda"] = np.minimum(reference["ka"], K)
    plot = (ggplot()
            + geom_line(reference, aes("ka", "lambda"), color=INK, linetype="dashed", size=0.6)
            + geom_point(points, aes("ka", "lambda"), color=CATEGORICAL[1], size=2.4)
            + annotate("text", x=0.6, y=K * 1.06, label="k = CL/V = 0.04 h⁻¹", color=INK,
                       ha="center", va="bottom", size=9)
            + scale_x_log10(breaks=[0.005, 0.01, 0.02, 0.04, 0.1, 0.2, 0.5, 1, 2])
            + labs(x="Absorption rate constant KA (h⁻¹, log scale)",
                   y="Slope of −ln C, 144–168 h (h⁻¹)")
            + style(width=7.0, height=3.8))
    return save(plot, "single-dose-terminal-slope")


def main():
    model = qq.CompiledModel(MODEL)
    for figure in (profiles, sampling, absorption_map, terminal_slope):
        print(figure(model).name)


if __name__ == "__main__":
    main()
