"""Figures for the population pharmacokinetics tutorial.

Reads the output directory written by population_pk.py, checks every plotted
value against the saved fit, VPC and bootstrap results, and writes five PNG
figures to docs/tutorials/figures/. Requires the ``docs`` extra (pandas and
plotnine).
"""

import argparse
import json
import math
from pathlib import Path

import numpy as np
import pandas as pd
from plotnine import (
    aes, facet_wrap, geom_abline, geom_hline, geom_jitter, geom_line, geom_point,
    geom_ribbon, geom_segment, geom_smooth, geom_vline, ggplot, labs, scale_color_manual,
    scale_shape_manual, scale_x_log10, scale_y_log10,
)

from figure_style import CATEGORICAL, FILL_LIGHT, INK, save, style

PERCENTILES = ["5th", "50th", "95th"]


def load(directory, name):
    return json.loads((directory / name).read_text(encoding="utf-8"))


def gof_table(directory, fit):
    """Goodness-of-fit rows, checked against the fitted observations of the fit."""
    gof = pd.read_csv(directory / "gof.csv", dtype={"ID": str})
    fitted = {r["source_row"]: r for r in fit["fitted_observations"]}
    assert len(gof) == len(fitted) == 600
    for row in gof.itertuples():
        reference = fitted[row.SOURCE_ROW]
        assert math.isclose(row.DV, reference["observed"], rel_tol=1e-12)
        assert math.isclose(row.PRED, reference["prediction"], rel_tol=1e-12)
        assert math.isclose(row.IPRED, reference["individual_prediction"], rel_tol=1e-12)
        assert math.isclose(row.CRES, reference["standardized_residual"], rel_tol=1e-12,
                            abs_tol=1e-12)
    return gof


def gof_figures(gof):
    """Figures 1 and 2: observations against predictions; residuals."""
    concentration = pd.concat([
        pd.DataFrame({"x": gof["PRED"], "y": gof["DV"],
                      "panel": "A  Population prediction (PRED)"}),
        pd.DataFrame({"x": gof["IPRED"], "y": gof["DV"],
                      "panel": "B  Individual prediction (IPRED)"}),
    ])
    top = (ggplot(concentration, aes("x", "y"))
           + geom_abline(intercept=0, slope=1, color=INK, linetype="dashed", size=0.5)
           + geom_point(color=CATEGORICAL[0], alpha=0.5, size=1.3, stroke=0)
           + geom_smooth(method="lowess", color=CATEGORICAL[1], se=False, size=0.9)
           + scale_x_log10() + scale_y_log10()
           + facet_wrap("~panel", ncol=2)
           + labs(x="Prediction (mg/L, log scale)",
                  y="Observed concentration (mg/L, log scale)")
           + style(width=7.5, height=4.0))
    residual = pd.concat([
        pd.DataFrame({"x": gof["TIME"], "y": gof["CRES"], "panel": "A  Time after dose (h)"}),
        pd.DataFrame({"x": gof["PRED"], "y": gof["CRES"], "panel": "B  PRED (mg/L)"}),
    ])
    bottom = (ggplot(residual, aes("x", "y"))
              + geom_hline(yintercept=0, color=INK, size=0.5)
              + geom_hline(yintercept=[-1.96, 1.96], color=INK, linetype="dotted", size=0.5)
              + geom_point(color=CATEGORICAL[0], alpha=0.5, size=1.3, stroke=0)
              + geom_smooth(method="lowess", color=CATEGORICAL[1], se=False, size=0.9)
              + facet_wrap("~panel", ncol=2, scales="free_x")
              + labs(x="", y="Conditional standardized residual")
              + style(width=7.5, height=4.0))
    return (save(top, "population-pk-gof-concentrations"),
            save(bottom, "population-pk-gof-residuals"))


def weight_figure(fit, gof, summary):
    """Figure 3: individual CL/F against body weight with the power functions."""
    estimate, truth = summary["python_saem"]["estimates"], summary["simulated"]
    assert math.isclose(estimate["cl"], fit["parameters"]["cl"]["value"])
    assert math.isclose(estimate["wt_exponent"], fit["covariate_effects"][0]["estimate"])
    weights = gof.groupby("ID")["WT"].first()
    points = pd.DataFrame([
        {"WT": float(weights[str(subject)]), "CL": individual["cl"]}
        for subject, individual in zip(fit["subject_ids"], fit["individual_parameters"])])
    assert len(points) == summary["design"]["subjects"]
    grid = np.geomspace(points["WT"].min(), points["WT"].max(), 60)
    lines = pd.concat([
        pd.DataFrame({"WT": grid, "CL": estimate["cl"] * (grid / 70) ** estimate["wt_exponent"],
                      "relation": f"Estimated (β = {estimate['wt_exponent']:.2f})"}),
        pd.DataFrame({"WT": grid, "CL": truth["cl"] * (grid / 70) ** truth["wt_exponent"],
                      "relation": f"Simulated (β = {truth['wt_exponent']:.2f})"}),
    ])
    plot = (ggplot()
            + geom_vline(xintercept=70, color=INK, linetype="dotted", size=0.4)
            + geom_point(points, aes("WT", "CL"), color=CATEGORICAL[0], size=2.0, alpha=0.8,
                         stroke=0)
            + geom_line(lines, aes("WT", "CL", color="relation", linetype="relation"), size=0.9)
            + scale_color_manual(values=[CATEGORICAL[1], INK])
            + scale_x_log10(breaks=[50, 60, 70, 80, 100, 120])
            + scale_y_log10()
            + labs(x="Body weight (kg, log scale)", y="Individual CL/F (L/h, log scale)",
                   color="", linetype="")
            + style(width=7.0, height=4.4))
    return save(plot, "population-pk-weight")


def vpc_figure(vpc, gof):
    """Figure 4: observed percentiles (points) and simulated percentiles (lines, ribbons)."""
    rows = []
    for b in vpc["bins"]:
        inside = gof["TIME"][(gof["TIME"] >= b["start"]) & (gof["TIME"] <= b["end"])]
        assert inside.nunique() == 1 and len(inside) == b["observations"]
        for item in b["quantiles"]:
            low, high = item["confidence_interval"]
            assert low <= item["simulated"] <= high
            rows.append({"time": float(inside.iloc[0]),
                         "percentile": f"{round(item['percentile'] * 100)}th",
                         "observed": item["observed"], "simulated": item["simulated"],
                         "low": low, "high": high})
    frame = pd.DataFrame(rows)
    assert len(frame) == 3 * len(vpc["bins"])
    frame["percentile"] = pd.Categorical(frame["percentile"], PERCENTILES)
    kinds = ["5th and 95th percentiles", "50th percentile"]
    frame["kind"] = pd.Categorical(
        np.where(frame["percentile"] == "50th", kinds[1], kinds[0]), kinds)
    plot = (ggplot(frame, aes("time", group="percentile"))
            + geom_ribbon(aes(ymin="low", ymax="high"), fill=FILL_LIGHT, alpha=0.35)
            + geom_line(aes(y="simulated", color="kind"), size=0.8)
            + geom_point(aes(y="observed", color="kind"), size=2.3, fill="white",
                         stroke=1.0, shape="o")
            + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[1]], breaks=kinds)
            + scale_x_log10(breaks=[0.5, 1, 2, 4, 8, 12, 24, 48])
            + scale_y_log10()
            + labs(x="Time after dose (h, log scale)",
                   y="Prediction-corrected concentration (mg/L, log scale)")
            + style(width=7.0, height=4.6))
    return save(plot, "population-pk-vpc")


def bootstrap_values(record):
    omega = record["omega"]
    return {"cl": record["parameters"]["cl"]["value"], "v": record["parameters"]["v"]["value"],
            "ka": record["parameters"]["ka"]["value"],
            "wt_exponent": record["covariate_effects"][0]["estimate"],
            "omega_cl": omega[0][0], "omega_v": omega[1][1], "omega_ka": omega[2][2],
            "additive_sd": record["error"]["additive"],
            "proportional_sd": record["error"]["proportional"]}


def bootstrap_figure(batches, summary):
    """Figure 5: converged replicates as ratios to the original estimates."""
    labels = {"cl": "θ CL/F", "v": "θ V/F", "ka": "θ KA", "wt_exponent": "β",
              "omega_cl": "ω² CL/F", "omega_v": "ω² V/F", "omega_ka": "ω² KA",
              "additive_sd": "σ add", "proportional_sd": "σ prop"}
    order = list(labels.values())[::-1]
    estimate, simulated = summary["python_saem"]["estimates"], summary["simulated"]

    def flat(values):
        return {**{k: values[k] for k in ("cl", "v", "ka", "wt_exponent", "additive_sd",
                                          "proportional_sd")},
                **{f"omega_{k}": v for k, v in values["omega"].items()}}

    reference, truth = flat(estimate), flat(simulated)
    records = [r for batch in batches for r in batch["replicates"]]
    converged = [bootstrap_values(r) for r in records if r["status"] == "converged"]
    assert len(converged) == summary["bootstrap"]["converged"]
    saved = summary["bootstrap"]["percentiles_2.5_50_97.5"]
    for name in labels:
        recomputed = np.quantile([v[name] for v in converged], [0.025, 0.5, 0.975])
        assert np.allclose(recomputed, saved[name], rtol=1e-12)
    points = pd.DataFrame([{"parameter": labels[k], "ratio": v[k] / reference[k]}
                           for v in converged for k in labels])
    bands = pd.DataFrame([{"parameter": labels[k], "low": saved[k][0] / reference[k],
                           "high": saved[k][2] / reference[k]} for k in labels])
    marks = pd.DataFrame([{"parameter": labels[k], "ratio": truth[k] / reference[k],
                           "mark": "Simulation value"} for k in labels])
    for frame in (points, bands, marks):
        frame["parameter"] = pd.Categorical(frame["parameter"], order)
    plot = (ggplot(points, aes("ratio", "parameter"))
            + geom_vline(xintercept=1, color=INK, size=0.5)
            + geom_segment(bands, aes(x="low", xend="high", y="parameter", yend="parameter"),
                           color=FILL_LIGHT, size=5, alpha=0.5)
            + geom_jitter(color=CATEGORICAL[0], alpha=0.85, size=1.7, stroke=0, height=0.12,
                          width=0, random_state=11)
            + geom_point(marks, aes(shape="mark"), color=INK, fill=CATEGORICAL[2], size=3.2,
                         stroke=0.6)
            + scale_shape_manual(values=["D"])
            + scale_x_log10(breaks=[0.05, 0.1, 0.25, 0.5, 1, 2])
            + labs(x="Replicate estimate ÷ estimate from the original data (log scale)", y="",
                   shape="")
            + style(width=7.0, height=4.8))
    return save(plot, "population-pk-bootstrap")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="directory written by population_pk.py")
    args = parser.parse_args()
    summary, fit = load(args.output, "summary.json"), load(args.output, "saem-fit.json")
    gof = gof_table(args.output, fit)
    paths = [*gof_figures(gof), weight_figure(fit, gof, summary),
             vpc_figure(load(args.output, "vpc.json"), gof),
             bootstrap_figure(load(args.output, "bootstrap.json"), summary)]
    for path in paths:
        print(path.name)


if __name__ == "__main__":
    main()
