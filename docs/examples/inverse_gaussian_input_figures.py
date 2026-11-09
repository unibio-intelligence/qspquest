"""Figures for the inverse Gaussian input tutorial.

Fits inverse-gaussian.qsp to inverse-gaussian-fit.json through the Python
binding, compares the simulated input and concentrations with closed forms, and
writes two PNG figures to docs/tutorials/figures/. Requires the ``docs`` extra
(numpy, scipy, pandas and plotnine).
"""

from copy import deepcopy
import json
import math
from pathlib import Path

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, facet_wrap, geom_hline, geom_line, geom_point, geom_vline, ggplot, labs,
    scale_color_manual, scale_linetype_manual, scale_x_continuous,
)
from scipy.integrate import quad
from scipy.stats import norm

import qspquest as qq
from figure_style import CATEGORICAL, INK, MUTED, save, style

HERE = Path(__file__).resolve().parent
MODEL = (HERE / "inverse-gaussian.qsp").read_text()
REQUEST = json.loads((HERE / "inverse-gaussian-fit.json").read_text())
DOSE, MEAN, SHAPE, K, VOLUME = 100.0, 2.0, 4.0, 0.2, 5.0  # mg, h, h, 1/h, L
COMPLETE, TRUTH, SD = 1.0, 0.7, 0.1
TOLERANCE = 1e-6  # mg/L, between the BDF solution and the quadrature solution
FITTED, FULL = "f = 0.7 (estimate)", "f = 1 (complete input)"


def density(t):
    """Inverse Gaussian density with mean MEAN and shape SHAPE (1/h)."""
    if t <= 0:
        return 0.0
    return math.sqrt(SHAPE / (2 * math.pi * t ** 3)) * math.exp(
        -SHAPE * (t - MEAN) ** 2 / (2 * MEAN ** 2 * t))


def cdf(t):
    """Closed-form inverse Gaussian distribution function [Chhikara and Folks]."""
    if t <= 0:
        return 0.0
    root = math.sqrt(SHAPE / t)
    return (norm.cdf(root * (t / MEAN - 1))
            + math.exp(2 * SHAPE / MEAN) * norm.cdf(-root * (t / MEAN + 1)))


def concentration(t, f):
    """C(t) = f*D/V * integral of g(s) exp(-k(t - s)) from 0 to t, by quadrature."""
    if t <= 0:
        return 0.0
    value, _ = quad(lambda s: density(s) * math.exp(-K * (t - s)), 0, t,
                    epsabs=1e-13, epsrel=1e-12, limit=200)
    return f * DOSE / VOLUME * value


def simulate(model, f, times):
    run = deepcopy(REQUEST["run"])
    run["parameters"] = {"f": {"value": f, "unit": "1"}}
    run["regimen"]["observations"] = [{"time": {"value": float(t), "unit": "h"}, "side": "post"}
                                      for t in times]
    result = model.run(run)
    (cp,) = [output for output in result["outputs"] if output["name"] == "cp"]
    assert cp["unit"] == "mg/L"
    return np.asarray(result["times"]), np.asarray(cp["values"])


def observations():
    times = [o["time"]["value"] for o in REQUEST["run"]["regimen"]["observations"]]
    rows = [{"time": times[o["row"]], "conc": o["value"]["value"]} for o in REQUEST["observations"]]
    for o in REQUEST["observations"]:
        assert o["error"]["additive_sd"]["value"] == SD and o["error"]["proportional_sd"] == 0
    return pd.DataFrame(rows)


def checks(model):
    """Fit, then verify the estimate and simulation against closed forms."""
    assert abs(quad(density, 0, np.inf, limit=200)[0] - 1) < 1e-9
    for t in (0.5, 1, 2, 4, 8, 16):
        assert abs(quad(density, 0, t, limit=200)[0] - cdf(t)) < 1e-10
    fit = model.fit_scalar(REQUEST)
    estimate = fit["parameters"]["f"]["value"]
    assert fit["status"] == "converged" and abs(estimate - TRUTH) <= 1e-5
    data = observations()
    # Concentration is proportional to f, so the least-squares estimate has a closed form.
    unit = np.array([concentration(t, 1.0) for t in data["time"]])
    closed = float(np.dot(data["conc"], unit) / np.dot(unit, unit))
    assert abs(estimate - closed) <= 1e-5
    assert max(abs(data["conc"] - TRUTH * unit)) < TOLERANCE
    constant = len(data) * (math.log(SD) + 0.5 * math.log(2 * math.pi))
    nll = lambda f: float(np.sum((data["conc"] - f * unit) ** 2) / (2 * SD ** 2) + constant)
    assert abs(nll(estimate) - fit["objective"]["negative_log_likelihood"]) < 1e-6
    return estimate, data, unit, nll


def input_figure(model, estimate, data):
    """Figure 1: input rate, cumulative input and concentration on one time axis."""
    times = np.round(np.linspace(0, 16, 641), 6)
    panels = ["A  Input rate (mg/h)", "B  Cumulative amount entered (mg)",
              "C  Plasma concentration (mg/L)"]
    lines = []
    for f, label in ((estimate, FITTED), (COMPLETE, FULL)):
        t, c = simulate(model, f, times)
        for x in (0.5, 1, 2, 4, 8, 16):
            index = int(np.argmin(abs(t - x)))
            assert abs(c[index] - concentration(x, f)) < TOLERANCE, (f, x)
        # Mass balance: amount entered = amount in the body + amount eliminated.
        eliminated = K * VOLUME * np.concatenate([[0], np.cumsum(np.diff(t) * (c[1:] + c[:-1]) / 2)])
        entered = VOLUME * c + eliminated
        expected = np.array([f * DOSE * cdf(x) for x in t])
        assert max(abs(entered - expected)) < 0.05, f
        rate = np.array([f * DOSE * density(x) for x in t])
        lines += [pd.DataFrame({"time": t, "value": rate, "panel": panels[0], "fit": label}),
                  pd.DataFrame({"time": t, "value": expected, "panel": panels[1], "fit": label}),
                  pd.DataFrame({"time": t, "value": c, "panel": panels[2], "fit": label})]
    lines = pd.concat(lines)
    lines["panel"] = pd.Categorical(lines["panel"], panels)
    lines["fit"] = pd.Categorical(lines["fit"], [FITTED, FULL])
    points = data.assign(panel=pd.Categorical([panels[2]] * len(data), panels))
    plateau = pd.DataFrame({"value": [estimate * DOSE, COMPLETE * DOSE],
                            "panel": pd.Categorical([panels[1]] * 2, panels)})
    plot = (ggplot()
            + geom_hline(plateau, aes(yintercept="value"), color=MUTED, linetype="dotted", size=0.4)
            + geom_line(lines, aes("time", "value", color="fit", linetype="fit"), size=0.8)
            + geom_point(points, aes("time", "conc"), color=INK, fill="white", shape="o",
                         size=2.4, stroke=0.9)
            + facet_wrap("~panel", ncol=1, scales="free_y")
            + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[1]])
            + scale_linetype_manual(values=["solid", "dashed"])
            + scale_x_continuous(breaks=[0, 2, 4, 6, 8, 10, 12, 14, 16])
            + labs(x="Time (h)", y="")
            + style(width=6.6, height=7.0))
    return save(plot, "inverse-gaussian-input-profiles")


def profile_figure(model, estimate, unit, nll):
    """Figure 2: negative log-likelihood as a function of f."""
    information = float(np.dot(unit, unit)) / SD ** 2
    half_width = 1.959964 / math.sqrt(information)
    grid = np.linspace(0.67, 0.73, 241)
    curve = pd.DataFrame({"f": grid, "delta": [nll(f) - nll(estimate) for f in grid]})
    data = observations()
    rows = []
    for f in np.linspace(0.675, 0.725, 11):
        t, c = simulate(model, float(f), [0] + list(data["time"]))
        value = np.sum((data["conc"].to_numpy() - c[1:]) ** 2) / (2 * SD ** 2)
        rows.append({"f": f, "delta": value - (nll(estimate) - len(data) * (
            math.log(SD) + 0.5 * math.log(2 * math.pi)))})
    points = pd.DataFrame(rows)
    assert max(abs(points["delta"] - [nll(f) - nll(estimate) for f in points["f"]])) < 1e-3
    cut = 0.5 * 1.959964 ** 2
    plot = (ggplot()
            + geom_hline(yintercept=cut, color=MUTED, linetype="dotted", size=0.5)
            + geom_vline(xintercept=[estimate - half_width, estimate + half_width], color=MUTED,
                         linetype="dotted", size=0.5)
            + geom_line(curve, aes("f", "delta"), color=CATEGORICAL[0], size=0.8)
            + geom_point(points, aes("f", "delta"), color=INK, fill="white", shape="o",
                         size=2.4, stroke=0.9)
            + annotate("text", x=0.6705, y=cut + 0.6, ha="left", va="bottom", color=INK, size=8,
                       label="ΔNLL = 1.92")
            + annotate("text", x=estimate, y=4.5, ha="center", va="center", color=INK, size=8,
                       label=f"95% interval\n{estimate - half_width:.4f}–{estimate + half_width:.4f}")
            + scale_x_continuous(breaks=[0.67, 0.68, 0.69, 0.70, 0.71, 0.72, 0.73])
            + labs(x="Fraction entering the systemic circulation, f",
                   y="Negative log-likelihood − minimum")
            + style(width=6.0, height=3.8))
    return save(plot, "inverse-gaussian-input-likelihood"), half_width


def main():
    model = qq.CompiledModel(MODEL)
    estimate, data, unit, nll = checks(model)
    print(input_figure(model, estimate, data).name)
    path, half_width = profile_figure(model, estimate, unit, nll)
    print(path.name, f"estimate {estimate:.6f}, 95% half-width {half_width:.4f}")


if __name__ == "__main__":
    main()
