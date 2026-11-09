"""Figures for the profiled proportional residual error tutorial.

Fits profiled-proportional.qsp with profiled-proportional-fit.json, as in the
tutorial's Python example, evaluates the native Gaussian objective on grids of
the model parameter and the residual scales, checks every value against the
closed-form likelihood, and writes two PNG figures to docs/tutorials/figures/.
Requires the ``docs`` extra (pandas and plotnine).
"""

import copy
import json
import math
from pathlib import Path

import numpy as np
import pandas as pd
from plotnine import (
    aes, coord_cartesian, facet_wrap, geom_hline, geom_line, geom_point, geom_ribbon, geom_text, geom_vline,
    ggplot, guide_legend, guides, labs, scale_color_manual, scale_fill_manual,
    scale_linetype_manual, scale_x_continuous, scale_x_log10, scale_y_continuous, theme,
)
from plotnine.composition import Beside

import qspquest
from figure_style import CATEGORICAL, FILL_LIGHT, INK, MUTED, save, style

HERE = Path(__file__).resolve().parent
MULTIPLIER = {"y1": 1.0, "y2": 2.0}
DOCUMENTED_SCALES = {"y1": 0.08944272, "y2": 0.14422205}
UNIT = "mg/L"


def load():
    source = (HERE / "profiled-proportional.qsp").read_text()
    request = json.loads((HERE / "profiled-proportional-fit.json").read_text())
    return qspquest.CompiledSensitivities(source, ["theta"]), request


def observed(request):
    rows = request["problem"]["request"]["objective"]["observations"]
    times = [t["value"] for t in request["problem"]["request"]["objective"]["simulation"]["run"][
        "regimen"]["samples"]]
    return pd.DataFrame([{"output": row["output"], "time": times[row["row"]],
                          "value": row["value"]["value"]} for row in rows])


def conditional_scales(data, theta):
    """Closed-form maximum-likelihood scale of each output, s = sqrt(mean((Y/mu - 1)^2))."""
    return {output: math.sqrt(np.mean((group["value"] / (MULTIPLIER[output] * theta) - 1) ** 2))
            for output, group in data.groupby("output")}


def analytic_nll(data, theta, scales):
    total = 0.0
    for row in data.itertuples():
        mu = MULTIPLIER[row.output] * theta
        sd = scales[row.output] * abs(mu)
        total += 0.5 * math.log(2 * math.pi) + math.log(sd) + (row.value - mu) ** 2 / (2 * sd ** 2)
    return total


def native_nll(model, request, theta, scales):
    """Native Gaussian objective with fixed proportional scales."""
    objective = copy.deepcopy(request["problem"]["request"]["objective"])
    objective["simulation"]["run"]["parameters"] = {"theta": {"value": theta, "unit": UNIT}}
    for row in objective["observations"]:
        row["error"]["proportional_sd"] = scales[row["output"]]
    return model.objective(objective)["objective"]


def checked():
    model, request = load()
    result = model.fit(request)
    fit = result["fit"]
    assert fit["status"] == "converged"
    data = observed(request)
    assert len(data) == fit["objective"]["observations"] == 8
    theta = fit["parameters"]["theta"]["value"]
    means = data.groupby("output")["value"].mean()
    assert math.isclose(theta, 2.5, rel_tol=1e-9)
    assert math.isclose(means["y1"], 2.5, rel_tol=1e-12)
    assert math.isclose(means["y2"] / 2, 2.5, rel_tol=1e-12)
    scales = result["residual_scales"]
    closed = conditional_scales(data, theta)
    for output, value in DOCUMENTED_SCALES.items():
        assert abs(scales[output] - value) < 5e-9, output
        assert math.isclose(scales[output], closed[output], rel_tol=1e-9), output
    minimum = fit["objective"]["objective"]
    assert f"{minimum:.3f}" == "4.052"  # stated in the Figure 1 caption
    assert math.isclose(minimum, analytic_nll(data, theta, scales), rel_tol=1e-10)
    assert math.isclose(minimum, native_nll(model, request, theta, scales), rel_tol=1e-10)
    return model, request, data, theta, scales, minimum


def profiles(model, request, data, theta_hat, scales, minimum):
    """Figure 1: objective against each residual scale, and the profile over theta."""
    rows, points = [], []
    for output in ("y1", "y2"):
        for s in np.geomspace(0.04, 0.5, 120):
            trial = dict(scales, **{output: float(s)})
            rows.append({"output": output, "s": s, "nll": analytic_nll(data, theta_hat, trial)})
        for s in np.geomspace(0.05, 0.45, 13):
            trial = dict(scales, **{output: float(s)})
            value = native_nll(model, request, theta_hat, trial)
            assert math.isclose(value, analytic_nll(data, theta_hat, trial), rel_tol=1e-10)
            points.append({"output": output, "s": s, "nll": value})
    curves, native = pd.DataFrame(rows), pd.DataFrame(points)
    optimum = pd.DataFrame([{"output": o, "s": scales[o], "nll": minimum,
                             "label": f"ŝ({o}) = {scales[o]:.4f}",
                             "x": scales[o] * (0.93 if o == "y1" else 1.07),
                             "ha": "right" if o == "y1" else "left"} for o in ("y1", "y2")])
    for frame in (curves, native, optimum):
        frame["output"] = pd.Categorical(frame["output"], ["y1", "y2"])
    by_scale = (ggplot()
                + geom_line(curves, aes("s", "nll", color="output"), size=0.8)
                + geom_point(native, aes("s", "nll", color="output"), fill="white", shape="o",
                             size=2.0, stroke=0.8)
                + geom_point(optimum, aes("s", "nll"), color=INK, size=2.8, shape="D")
                + geom_text(optimum[optimum["output"] == "y1"],
                            aes("x", "nll", label="label", color="output"), ha="right",
                            va="top", nudge_y=-0.4, size=8.5, show_legend=False)
                + geom_text(optimum[optimum["output"] == "y2"],
                            aes("x", "nll", label="label", color="output"), ha="left",
                            va="top", nudge_y=-0.4, size=8.5, show_legend=False)
                + scale_color_manual(values=CATEGORICAL[:2],
                                     labels=["s(y1) varied, s(y2) = ŝ(y2)",
                                             "s(y2) varied, s(y1) = ŝ(y1)"])
                + guides(color=guide_legend(ncol=1))
                + scale_x_log10(breaks=[0.05, 0.1, 0.2, 0.3, 0.5])
                + coord_cartesian(ylim=(minimum - 2, minimum + 12))
                + labs(x="Residual scale s (log scale)",
                       y="Negative log-likelihood",
                       title="A  Residual scales at θ = 2.5 mg/L")
                + style(width=4.0, height=4.4))
    thetas = np.linspace(2.0, 3.0, 101)
    profile = pd.DataFrame({"theta": thetas, "nll": [
        analytic_nll(data, t, conditional_scales(data, t)) for t in thetas]})
    native_rows = []
    for t in np.linspace(2.05, 2.95, 10):
        conditional = conditional_scales(data, float(t))
        value = native_nll(model, request, float(t), conditional)
        assert math.isclose(value, analytic_nll(data, float(t), conditional), rel_tol=1e-10)
        native_rows.append({"theta": t, "nll": value})
    native_profile = pd.DataFrame(native_rows)
    by_theta = (ggplot()
                + geom_vline(xintercept=theta_hat, color=MUTED, linetype="dashed", size=0.5)
                + geom_line(profile, aes("theta", "nll"), color=INK, size=0.8)
                + geom_point(native_profile, aes("theta", "nll"), color=INK, fill="white",
                             shape="o", size=2.0, stroke=0.8)
                + geom_point(pd.DataFrame({"theta": [theta_hat], "nll": [minimum]}),
                             aes("theta", "nll"), color=INK, size=2.8, shape="D")
                + geom_text(pd.DataFrame({"theta": [theta_hat + 0.03], "nll": [minimum - 0.6],
                                          "label": [f"θ̂ = 2.5 mg/L, objective {minimum:.3f}"]}),
                            aes("theta", "nll", label="label"), ha="left", color=INK, size=8.5)
                + scale_x_continuous(breaks=[2.0, 2.25, 2.5, 2.75, 3.0])
                + scale_y_continuous(limits=(minimum - 1.5, None))
                + labs(x="θ (mg/L)", y="Profiled negative log-likelihood",
                       title="B  Profile over θ")
                + style(width=4.0, height=4.4))
    plot = Beside([by_scale, by_theta]) + theme(figure_size=(8.0, 4.6))
    return save(plot, "profiled-proportional-errors-objective")


def fitted(data, theta_hat, scales):
    """Figure 2: observations, fitted predictions and estimated residual intervals."""
    grid = pd.DataFrame([{"output": o, "time": t, "mu": MULTIPLIER[o] * theta_hat,
                          "low": MULTIPLIER[o] * theta_hat * (1 - 1.959963984540054 * scales[o]),
                          "high": MULTIPLIER[o] * theta_hat * (1 + 1.959963984540054 * scales[o])}
                         for o in ("y1", "y2") for t in (0.0, 4.0)])
    labels = {o: f"{o} = {'θ' if o == 'y1' else '2θ'}, ŝ = {scales[o]:.4f}" for o in ("y1", "y2")}
    for frame in (grid, data):
        frame["panel"] = pd.Categorical([labels[o] for o in frame["output"]], list(labels.values()))
    plot = (ggplot()
            + geom_ribbon(grid, aes("time", ymin="low", ymax="high",
                                    fill="'Prediction ± 1.96 ŝ·μ'"), alpha=0.3)
            + geom_line(grid, aes("time", "mu", linetype="'Fitted prediction μ'"), color=INK,
                        size=0.8)
            + geom_point(data, aes("time", "value", color="'Observation'"), size=2.6)
            + facet_wrap("~panel", ncol=2, scales="free_y")
            + scale_fill_manual(values=[FILL_LIGHT])
            + scale_color_manual(values=[CATEGORICAL[1]])
            + scale_linetype_manual(values=["solid"])
            + scale_x_continuous(breaks=[0, 1, 2, 3, 4])
            + guides(color=guide_legend(order=1), linetype=guide_legend(order=2),
                     fill=guide_legend(order=3))
            + labs(x="Time (h)", y="Concentration (mg/L)")
            + style(width=7.0, height=3.8))
    return save(plot, "profiled-proportional-errors-fit")


def main():
    model, request, data, theta, scales, minimum = checked()
    print(profiles(model, request, data, theta, scales, minimum).name)
    print(fitted(data, theta, scales).name)


if __name__ == "__main__":
    main()
