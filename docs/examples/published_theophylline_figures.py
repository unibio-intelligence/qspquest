"""Figures for the published theophylline tutorial (published-theophylline.md).

Runs published_theophylline.py against the pinned validation companion in a
temporary directory, checks the fit against the values reported in the
tutorial and the CLI predictions against the analytical solution, and writes
three PNG figures to docs/tutorials/figures/. The profile likelihoods are
calculated with the analytical solution of the linear model, after checking
that it reproduces the CLI predictions and objective. Requires the ``docs`` extra
(numpy, scipy, pandas and plotnine), a built CLI and the validation companion
at the pinned revision.
"""

import argparse
import csv
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
import pandas as pd
from plotnine import (
    aes, facet_wrap, geom_hline, geom_line, geom_point, geom_rect, ggplot, labs,
    scale_color_manual, scale_shape_manual, scale_x_continuous, scale_x_log10,
    scale_y_continuous, scale_y_log10, theme,
)
from plotnine.composition import Beside
from scipy.optimize import minimize

from figure_style import CATEGORICAL, FILL_LIGHT, INK, save, style
from published_theophylline import VALIDATION_COMMIT

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SD = 2.1  # fixed additive residual standard deviation
DOCUMENTED = {"cl": 1.58591, "vc": 29.39343, "ka": 1.77741}  # Results table
NLL = 17.03261
NAMES = ["cl", "vc", "ka"]
LABELS = {"cl": "cl (1/h)", "vc": "vc (source scale)", "ka": "ka (1/h)"}
# 95% quantile of the chi-square distribution with 1 degree of freedom: the profile
# likelihood threshold for an approximate 95% confidence interval.
THRESHOLD = 3.841458820694124


def run_example(validation, cli, out):
    """Run the tutorial script exactly as documented and read its outputs."""
    process = subprocess.run(
        [sys.executable, "-B", str(HERE / "published_theophylline.py"), "--validation",
         str(validation), "--cli", str(cli), "--out", str(out)],
        capture_output=True, text=True, check=True)
    summary = json.loads(process.stdout)
    fit = json.loads((out / "fit.json").read_text())["fit"]
    receipt = json.loads((out / "receipt.json").read_text())
    with (out / "observed-predicted.csv").open() as stream:
        table = pd.DataFrame([{k: float(v) for k, v in row.items()} for row in csv.DictReader(stream)])
    with (out / "data.csv").open() as stream:
        dose = next(float(r["AMT"]) for r in csv.DictReader(stream) if r["EVID"] == "1")
    return summary, fit, receipt, table, dose


def check_fit(summary, fit, receipt, table):
    assert summary["status"] == "converged" and summary["observations"] == 10
    assert fit["status"] == "converged" and receipt["validation_commit"] == VALIDATION_COMMIT
    for name, value in DOCUMENTED.items():
        assert abs(fit["parameters"][name]["value"] / value - 1) < 1e-4, name
    assert abs(fit["objective"]["negative_log_likelihood"] - NLL) < 1e-4
    assert len(table) == 10 and table["time_h"].min() == 0.25 and table["time_h"].max() == 24.37
    row = table[table["time_h"] == 1.12].iloc[0]
    assert row["observed"] == 10.5 and abs(row["predicted"] - 9.0353) < 1e-4
    assert max(abs(table["residual"] - (table["observed"] - table["predicted"]))) < 1e-12


def bateman(t, dose, cl, vc, ka):
    """Analytical concentration after an oral dose (Bateman function)."""
    k = cl / vc
    if abs(ka - k) < 1e-12 * ka:  # limit for ka = k
        return dose * ka / vc * t * np.exp(-k * t)
    return dose * ka / (vc * (ka - k)) * (np.exp(-k * t) - np.exp(-ka * t))


def dense_prediction(cli, out, times, parameters=None):
    """Simulate the fitted model (or other parameter values) on a fine time grid with the CLI."""
    request = json.loads((out / "prediction-request.json").read_text())
    for name, value in (parameters or {}).items():
        request["parameters"][name]["value"] = value
    request["regimen"]["observations"] = [
        {"time": {"value": float(t), "unit": "h"}, "side": "post"} for t in times]
    path = out / "dense-request.json"
    path.write_text(json.dumps(request))
    process = subprocess.run([str(cli), "run", str(out / "model.qsp"), str(path)],
                             capture_output=True, text=True, check=True)
    result = json.loads(process.stdout)
    return np.asarray(next(c["values"] for c in result["outputs"] if c["name"] == "cp"))


def negative_log_likelihood(log_parameters, table, dose):
    cl, vc, ka = np.exp(log_parameters)
    residual = table["observed"].to_numpy() - bateman(table["time_h"].to_numpy(), dose, cl, vc, ka)
    return 0.5 * np.sum(np.log(2 * np.pi * SD ** 2) + (residual / SD) ** 2)


def mirror(estimate):
    """Flip-flop parameter set: ka and cl/vc interchanged, giving the same curve and cl."""
    k = estimate["cl"] / estimate["vc"]
    vc = estimate["vc"] * k / estimate["ka"]
    return {"cl": estimate["ka"] * vc, "vc": vc, "ka": k}


def profiles(table, dose, estimate, alternative):
    """Profile likelihood of each parameter: the other two are re-estimated at each value."""
    starts = [np.log([p[n] for n in NAMES]) for p in (estimate, alternative)]
    minimum = negative_log_likelihood(starts[0], table, dose)
    spans = {"cl": (-1.5, 1.5), "vc": (-4.5, 1.5), "ka": (-4.5, 1.5)}
    rows = []
    for index, name in enumerate(NAMES):
        centre = starts[0][index]
        grid = centre + np.linspace(*spans[name], 121)
        previous = None
        for value in grid:
            def objective(free):
                return negative_log_likelihood(np.insert(free, index, value), table, dose)

            candidates = [np.delete(s, index) for s in starts] + ([previous] if previous is not None else [])
            best = min((minimize(objective, c, method="Nelder-Mead",
                                 options={"xatol": 1e-9, "fatol": 1e-11, "maxiter": 4000})
                        for c in candidates), key=lambda r: r.fun)
            previous = best.x
            rows.append({"parameter": name, "value": math.exp(value),
                         "deviance": 2 * (best.fun - minimum)})
    frame = pd.DataFrame(rows)
    assert frame["deviance"].min() > -1e-6, "the reported estimate is not the optimum"
    return frame


def fit_figure(table, curve):
    linear = (ggplot()
              + geom_line(curve, aes("time", "conc", color="kind"), size=0.8)
              + geom_point(table.assign(kind="Observed"), aes("time_h", "observed", shape="kind"),
                           color=INK, size=2.4)
              + scale_color_manual(values=[CATEGORICAL[0]])
              + scale_shape_manual(values=["o"])
              + scale_x_continuous(breaks=[0, 4, 8, 12, 16, 20, 24])
              + scale_y_continuous(limits=(0, 11.5), breaks=[0, 2, 4, 6, 8, 10])
              + labs(x="Time after dose (h)", y="Concentration (source scale)",
                     title="A  Linear scale")
              + style(width=4.1, height=4.0) + theme(legend_position="none"))
    log = (ggplot()
           + geom_line(curve[curve["conc"] > 0.8], aes("time", "conc", color="kind"), size=0.8)
           + geom_point(table.assign(kind="Observed"), aes("time_h", "observed", shape="kind"),
                        color=INK, size=2.4)
           + scale_color_manual(values=[CATEGORICAL[0]])
           + scale_shape_manual(values=["o"])
           + scale_x_continuous(breaks=[0, 4, 8, 12, 16, 20, 24])
           + scale_y_log10(limits=(0.8, 15), breaks=[1, 2, 5, 10])
           + labs(x="Time after dose (h)", y="Concentration (source scale, log scale)",
                  title="B  Logarithmic scale")
           + style(width=4.1, height=4.0) + theme(legend_position="none"))
    plot = Beside([linear, log]) & theme(figure_size=(8.2, 4.0))
    return save(plot, "published-theophylline-fit")


def residual_figure(table):
    band = dict(ymin=-SD, ymax=SD, fill=FILL_LIGHT, alpha=0.25)

    def panel(x, xlabel, title, xlimits, breaks):
        return (ggplot(table, aes(x, "residual"))
                + geom_rect(aes(xmin=-math.inf, xmax=math.inf), data=table.head(1), **band)
                + geom_hline(yintercept=0, color=INK, size=0.5)
                + geom_point(color=CATEGORICAL[1], size=2.4)
                + scale_x_continuous(limits=xlimits, breaks=breaks)
                + scale_y_continuous(limits=(-3, 3), breaks=[-3, -2, -1, 0, 1, 2, 3])
                + labs(x=xlabel, y="Residual, observed − predicted", title=title)
                + style(width=4.1, height=3.6))

    plot = Beside([panel("time_h", "Time after dose (h)", "A  Against time", (0, 25),
                         [0, 4, 8, 12, 16, 20, 24]),
                   panel("predicted", "Predicted concentration (source scale)",
                         "B  Against prediction", (2, 11), [2, 4, 6, 8, 10])]) \
        & theme(figure_size=(8.2, 3.6))
    return save(plot, "published-theophylline-residuals")


def profile_figure(frame, estimate, alternative):
    order = [LABELS[n] for n in NAMES]
    frame = frame.assign(parameter=pd.Categorical(frame["parameter"].map(LABELS), order))
    points = pd.DataFrame(
        [{"parameter": LABELS[n], "value": p[n], "deviance": 0.0, "kind": kind}
         for kind, p in (("Estimate (CLI fit)", estimate),
                         ("Flip-flop solution", alternative)) for n in NAMES])
    points["parameter"] = pd.Categorical(points["parameter"], order)
    points["kind"] = pd.Categorical(points["kind"], ["Estimate (CLI fit)", "Flip-flop solution"])
    plot = (ggplot()
            + geom_hline(yintercept=THRESHOLD, color=INK, linetype="dashed", size=0.5)
            + geom_line(frame, aes("value", "deviance"), color=CATEGORICAL[0], size=0.8)
            + geom_point(points, aes("value", "deviance", shape="kind", color="kind"),
                         size=2.6, stroke=1.0, fill="white")
            + facet_wrap("~parameter", scales="free_x")
            + scale_x_log10()
            + scale_y_continuous(limits=(0, 12), breaks=[0, 2, 3.84, 6, 8, 10, 12],
                                 labels=["0", "2", "3.84", "6", "8", "10", "12"])
            + scale_color_manual(values=[INK, CATEGORICAL[1]])
            + scale_shape_manual(values=["o", "D"])
            + labs(x="Parameter value (log scale)",
                   y="2 × increase in negative log-likelihood")
            + style(width=8.4, height=3.6)
            + theme(panel_spacing=0.05))
    return save(plot, "published-theophylline-profiles")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validation", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/release/qspquest")
    args = parser.parse_args()
    cli = args.cli.resolve()
    with tempfile.TemporaryDirectory() as scratch:
        out = Path(scratch) / "theophylline-fit"
        summary, fit, receipt, table, dose = run_example(args.validation, cli, out)
        check_fit(summary, fit, receipt, table)
        estimate = {n: fit["parameters"][n]["value"] for n in NAMES}
        request = json.loads((out / "diffsol_bdf-request.json").read_text())
        run = request["problem"]["request"]["objective"]["simulation"]["run"]
        initial = {n: run["parameters"][n]["value"] for n in NAMES}
        times = np.linspace(0.0, 24.37, 244)
        conc = dense_prediction(cli, out, times)
        alternative = mirror(estimate)
        flipped = dense_prediction(cli, out, times, alternative)
    values = [estimate[n] for n in NAMES]
    # The CLI predictions agree with the analytical solution of the linear model.
    assert max(abs(conc - bateman(times, dose, *values))) < 1e-6
    assert max(abs(table["predicted"] - bateman(table["time_h"], dose, *values))) < 1e-6
    # The analytical likelihood reproduces the objective reported by the CLI.
    centre = np.log(values)
    assert abs(negative_log_likelihood(centre, table, dose)
               - fit["objective"]["negative_log_likelihood"]) < 1e-6
    # Stated in the tutorial: the flip-flop parameter set has the same cl, gives the
    # same CLI prediction and the same likelihood, with ka < cl/vc.
    assert abs(alternative["cl"] / estimate["cl"] - 1) < 1e-12
    assert alternative["ka"] < alternative["cl"] / alternative["vc"]
    assert max(abs(flipped - conc)) < 1e-6
    assert abs(negative_log_likelihood(np.log([alternative[n] for n in NAMES]), table, dose)
               - negative_log_likelihood(centre, table, dose)) < 1e-9
    assert round(alternative["vc"], 2) == 0.89 and round(alternative["ka"], 3) == 0.054
    # Stated in the tutorial: every residual is smaller in magnitude than the fixed SD,
    # and the largest, 1.46, is at 1.12 h.
    largest = table.loc[table["residual"].abs().idxmax()]
    assert table["residual"].abs().max() < SD
    assert largest["time_h"] == 1.12 and round(largest["residual"], 2) == 1.46
    # Stated in the tutorial: the initial estimates have ka > cl/vc, as does the estimate.
    assert initial["ka"] > initial["cl"] / initial["vc"]
    assert estimate["ka"] > estimate["cl"] / estimate["vc"]
    curve = pd.DataFrame({"time": times, "conc": conc, "kind": "Fitted prediction"})
    profile = profiles(table, dose, estimate, alternative)
    for path in (fit_figure(table, curve), residual_figure(table),
                 profile_figure(profile, estimate, alternative)):
        print(path.name)


if __name__ == "__main__":
    main()
