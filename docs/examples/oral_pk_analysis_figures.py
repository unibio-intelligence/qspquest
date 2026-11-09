"""Figures for the oral pharmacokinetic fitting tutorial (oral-pk-analysis.md).

Regenerates the 12 seeded synthetic subjects with oral_pk_analysis.py, repeats
the individual fits of the tutorial, checks the simulations against the
analytical solution and the fitted values against the fit results, and writes
four PNG figures to docs/tutorials/figures/. Requires the ``docs`` extra
(numpy, pandas and plotnine).
"""

import math
from pathlib import Path

import numpy as np
import pandas as pd
from plotnine import (
    aes, facet_wrap, geom_abline, geom_blank, geom_hline, geom_line, geom_point, geom_vline, ggplot, labs,
    scale_color_manual, scale_linetype_manual, scale_shape_manual, scale_x_continuous,
    scale_x_log10, scale_y_continuous, scale_y_log10, theme,
)
from plotnine.composition import Beside

import qspquest as qq
from figure_style import CATEGORICAL, INK, MUTED, save, style
from oral_pk_analysis import parameter_specs, synthetic_subjects

ROOT = Path(__file__).resolve().parents[2]
DOSE = 180.0  # mg, oral dose of every synthetic subject
ERROR = {"model": "normal", "additive": 0.2, "proportional": 0.1}
TOLERANCE = 1e-9  # mg/L, simulated against analytical concentrations
PARAMETERS = [("cl", "CL (L/h)", "L/h"), ("vc", "Vc (L)", "L"), ("ka", "ka (1/h)", "1/h")]


def bateman(t, cl, vc, ka):
    """Analytical concentration after an oral dose with first-order absorption."""
    k = cl / vc
    return DOSE / vc * ka / (ka - k) * (math.exp(-k * t) - math.exp(-ka * t))


def simulate(model, values, times):
    request = qq.run_request(
        end=24, times=list(times), administrations=[qq.bolus("depot", DOSE, "mg")],
        solver="analytic_linear_pk",
        parameters={name: qq.quantity(float(values[name]), unit) for name, _, unit in PARAMETERS})
    result = model.run(request)
    return np.asarray(next(c["values"] for c in result["outputs"] if c["name"] == "cp"))


def fit_all():
    """Repeat the tutorial: generate the data and fit every subject."""
    model, subjects, truth = synthetic_subjects(
        (ROOT / "examples/synthetic/absorption.qsp").read_text())
    assert len(subjects) == 12
    assert sum(len(s["observations"]) for s in subjects) == 96
    fits = [model.fit_data(s["run"], s["observations"], parameter_specs(), ERROR,
                           method="derivative_free", max_iterations=400, max_simulations=5000)
            for s in subjects]
    statuses = {f["status"] for f in fits}
    assert statuses <= {"converged", "evaluation_limit"}, statuses
    if statuses != {"converged"}:
        print("Fit status:", {s["id"]: f["status"] for s, f in zip(subjects, fits)})
    return model, subjects, truth, fits


def tables(model, subjects, truth, fits):
    """Observations, residuals, dense profiles and estimates, with checks."""
    dense = np.geomspace(0.1, 24, 160)
    rows, curves, estimates = [], [], []
    for subject, true, fit in zip(subjects, truth, fits):
        label = f"Subject {subject['id']}"
        if fit["status"] != "converged":
            label += "*"
        estimated = {name: fit["parameters"][name]["value"] for name, _, _ in PARAMETERS}
        observed = fit["fitted_observations"]
        times = [r["time"] for r in observed]
        # The fitted predictions are the model simulated at the estimates.
        assert max(abs(simulate(model, estimated, times) - [r["prediction"] for r in observed])) < 1e-12
        # Simulation at the generating parameters reproduces the analytical generator.
        exact = [bateman(t, true["cl"], true["vc"], true["ka"]) for t in dense]
        # Stated in the tutorial: four samples (0.25-2 h) precede the analytical Tmax.
        k = true["cl"] / true["vc"]
        tmax = math.log(true["ka"] / k) / (true["ka"] - k)
        assert sum(t < tmax for t in times) == 4 and times[3] == 2, tmax
        assert max(abs(simulate(model, true, dense) - exact)) < TOLERANCE
        for r in observed:
            assert abs(r["residual"] - (r["observed"] - r["prediction"])) < 1e-12
            sd = math.hypot(ERROR["additive"], ERROR["proportional"] * r["prediction"])
            assert abs(r["standardized_residual"] - r["residual"] / sd) < 1e-9
        assert qq.fit_diagnostics(fit)["residual_summary"]["count"] == len(observed)
        rows.append(pd.DataFrame({"subject": label, "time": times,
                                  "observed": [r["observed"] for r in observed],
                                  "prediction": [r["prediction"] for r in observed],
                                  "cwres": [r["standardized_residual"] for r in observed]}))
        curves.append(pd.DataFrame({"subject": label, "time": dense, "conc": exact,
                                    "kind": "Generating profile (analytical)"}))
        curves.append(pd.DataFrame({"subject": label, "time": dense,
                                    "conc": simulate(model, estimated, dense),
                                    "kind": "Individual prediction (fitted)"}))
        for name, title, _ in PARAMETERS:
            estimates.append({"subject": label, "parameter": title, "name": name,
                              "true": true[name], "estimate": estimated[name],
                              "status": fit["status"]})
    observations, curves, estimates = pd.concat(rows), pd.concat(curves), pd.DataFrame(estimates)
    order = list(dict.fromkeys(observations["subject"]))
    for frame in (observations, curves, estimates):
        frame["subject"] = pd.Categorical(frame["subject"], order)
    assert len(observations) == 96
    return observations, curves, estimates


def profiles(observations, curves):
    """Figure 1: observed and fitted profiles, one panel per subject."""
    kinds = ["Individual prediction (fitted)", "Generating profile (analytical)"]
    curves["kind"] = pd.Categorical(curves["kind"], kinds)
    plot = (ggplot()
            + geom_line(curves, aes("time", "conc", color="kind", linetype="kind"), size=0.7)
            + geom_point(observations.assign(point="Observed"),
                         aes("time", "observed", shape="point"), color=INK, size=1.6)
            + facet_wrap("~subject", ncol=4)
            + scale_y_log10(breaks=[0.5, 1, 2, 5])
            + scale_x_continuous(breaks=[0, 6, 12, 18, 24])
            + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[1]])
            + scale_linetype_manual(values=["solid", "dashed"])
            + scale_shape_manual(values=["o"])
            + labs(x="Time after dose (h)", y="Concentration (mg/L, log scale)")
            + style(width=8.0, height=6.4))
    return save(plot, "oral-pk-analysis-profiles")


def parameter_recovery(estimates):
    """Figure 2: estimated against generating parameters, with identity lines."""
    estimates["parameter"] = pd.Categorical(estimates["parameter"], [p[1] for p in PARAMETERS])
    estimates["status"] = pd.Categorical(estimates["status"], ["converged", "evaluation_limit"])
    # Equal ranges on both axes of each panel, so that the identity line is the diagonal.
    span = estimates.groupby("parameter", observed=True).apply(
        lambda f: pd.Series({"low": min(f["true"].min(), f["estimate"].min()) / 1.05,
                             "high": max(f["true"].max(), f["estimate"].max()) * 1.05}),
        include_groups=False).reset_index()
    corners = pd.concat([span.assign(true=span["low"], estimate=span["low"]),
                         span.assign(true=span["high"], estimate=span["high"])])
    plot = (ggplot(estimates, aes("true", "estimate"))
            + geom_blank(corners)
            + geom_abline(slope=1, intercept=0, color=INK, linetype="dashed", size=0.5)
            + geom_point(aes(color="status", shape="status"), size=2.4, stroke=0.9)
            + facet_wrap("~parameter", scales="free")
            + scale_x_log10() + scale_y_log10()
            + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[2]], drop=False)
            + scale_shape_manual(values=["o", "^"], drop=False)
            + labs(x="Generating value (log scale)", y="Estimate (log scale)")
            + style(width=8.0, height=3.3)
            + theme(panel_spacing=0.05))
    return save(plot, "oral-pk-analysis-parameters")


def goodness_of_fit(observations):
    """Figure 3: observed against predicted, and residuals against time and prediction."""
    limits = (0.8, 8)
    observed = (ggplot(observations, aes("prediction", "observed"))
                + geom_abline(slope=1, intercept=0, color=INK, linetype="dashed", size=0.5)
                + geom_point(color=CATEGORICAL[0], size=1.8, alpha=0.85)
                + scale_x_log10(limits=limits, breaks=[1, 2, 5])
                + scale_y_log10(limits=limits, breaks=[1, 2, 5])
                + labs(x="Individual prediction (mg/L, log scale)",
                       y="Observed (mg/L, log scale)", title="A  Observed and predicted")
                + style(width=2.8, height=3.2))

    def residuals(x, xlabel, title, scale):
        return (ggplot(observations, aes(x, "cwres"))
                + geom_hline(yintercept=0, color=INK, size=0.5)
                + geom_hline(yintercept=[-1.96, 1.96], color=MUTED, linetype="dashed", size=0.4)
                + geom_point(color=CATEGORICAL[0], size=1.8, alpha=0.85)
                + scale
                + scale_y_continuous(limits=(-4, 4), breaks=[-4, -2, 0, 2, 4])
                + labs(x=xlabel, y="Standardized residual", title=title)
                + style(width=2.8, height=3.2))

    against_time = residuals("time", "Time after dose (h)", "B  Residual and time",
                             scale_x_continuous(breaks=[0, 6, 12, 18, 24]))
    against_prediction = residuals("prediction", "Individual prediction (mg/L, log scale)",
                                   "C  Residual and prediction",
                                   scale_x_log10(limits=limits, breaks=[1, 2, 5]))
    assert observations["cwres"].abs().max() < 4
    plot = Beside([observed, against_time, against_prediction]) & theme(figure_size=(8.4, 3.4))
    return save(plot, "oral-pk-analysis-goodness-of-fit")


def error_correlation(estimates):
    """Figure 4: correlation of the estimation errors between parameters, across subjects."""
    ratio = (estimates.pivot(index="subject", columns="name", values="estimate")
             / estimates.pivot(index="subject", columns="name", values="true"))
    errors = np.log(ratio)
    labels = {"cl": "CL", "vc": "Vc", "ka": "ka"}
    panels = []
    for letter, (x, y) in zip("ABC", [("cl", "vc"), ("cl", "ka"), ("vc", "ka")]):
        r = float(np.corrcoef(errors[x], errors[y])[0, 1])
        panels.append(
            ggplot(errors, aes(x, y))
            + geom_hline(yintercept=0, color=MUTED, size=0.5)
            + geom_vline(xintercept=0, color=MUTED, size=0.5)
            + geom_point(color=CATEGORICAL[1], size=2.2)
            + scale_x_continuous(limits=(-0.5, 0.5)) + scale_y_continuous(limits=(-0.5, 0.5))
            + labs(x=f"ln({labels[x]} estimate / generating value)",
                   y=f"ln({labels[y]} estimate / generating value)",
                   title=f"{letter}  {labels[x]} and {labels[y]}, r = {r:.2f}")
            + style(width=2.8, height=3.0))
    assert errors.abs().max().max() < 0.5
    # Statements in the tutorial: the errors of vc and ka are positively correlated,
    # and the deviations from the generating values are largest for ka.
    corr = np.corrcoef(errors["vc"], errors["ka"])[0, 1]
    assert corr > 0.5, corr
    spread = errors.abs().mean()
    assert spread["ka"] > max(spread["cl"], spread["vc"]), spread
    plot = Beside(panels) & theme(figure_size=(8.4, 3.2))
    return save(plot, "oral-pk-analysis-error-correlation")


def main():
    model, subjects, truth, fits = fit_all()
    observations, curves, estimates = tables(model, subjects, truth, fits)
    for path in (profiles(observations, curves), parameter_recovery(estimates),
                 goodness_of_fit(observations), error_correlation(estimates)):
        print(path.name)


if __name__ == "__main__":
    main()
