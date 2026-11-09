"""Figures for the log-scale Wald interval tutorial.

Runs population_log_wald.py, repeats its SAEM fit in a temporary directory to
obtain the fitted observations, checks the intervals and predictions, and writes
three PNG figures to docs/tutorials/figures/. Requires the ``docs`` extra
(pandas and plotnine).
"""

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
    aes, annotate, facet_wrap, geom_abline, geom_hline, geom_jitter, geom_line, geom_point,
    geom_text, guide_legend, guides, scale_alpha_manual, scale_size_manual,
    geom_ribbon, geom_segment, geom_vline, ggplot, labs, position_jitter, scale_color_manual,
    scale_fill_manual, scale_linetype_manual, scale_x_continuous, scale_x_log10, scale_y_continuous,
    theme,
)
from plotnine.composition import Beside

from figure_style import CATEGORICAL, FILL_LIGHT, INK, MUTED, save, style
from population_two_effects import NAMES, prepare

SCRIPT = Path(__file__).resolve().parent / "population_log_wald.py"
Z = 1.959963984540054
METHOD = "positive_fixed_effect_log_wald_delta_v1"
LABELS = {"theta_intercept": "θ_intercept", "theta_slope": "θ_slope"}


def printed_intervals():
    """Run the tutorial script and parse the interval record it prints."""
    completed = subprocess.run([sys.executable, str(SCRIPT)], check=True, capture_output=True,
                               text=True, cwd=SCRIPT.parent)
    text = completed.stdout
    return json.loads(text[text.index("{"):text.rindex("}") + 1])


def fit():
    """Repeat the tutorial fit and keep the design and observations it uses."""
    from qspquest import CompiledSensitivities
    with tempfile.TemporaryDirectory(prefix="qspquest-log-wald-figures-") as temporary:
        directory = Path(temporary) / "population"
        prepare(directory)
        path = directory / "saem-fixed-request.json"
        request = json.loads(path.read_text())
        request["problem"]["request"]["uncertainty"] = True
        result = CompiledSensitivities((directory / "model.qsp").read_text(), NAMES).fit(request)
        design = json.loads((directory / "design.json").read_text())
        with (directory / "observations.csv").open() as stream:
            observed = [{key: float(value) for key, value in row.items()}
                        for row in csv.DictReader(stream)]
    return result["population"], design, pd.DataFrame(observed)


def checked():
    printed = printed_intervals()
    population, design, observed = fit()
    assert population["status"] == "converged"
    intervals = population["fixed_effect_intervals"]
    assert intervals == printed
    assert intervals["method"] == METHOD and intervals["confidence_level"] == 0.95
    assert intervals["critical_value"] == Z
    assert [row["name"] for row in intervals["intervals"]] == NAMES
    for row in intervals["intervals"]:
        estimate, se = row["estimate"], row["source_standard_error"]
        assert row["unit"] == "1" and 0 < row["lower"] < estimate < row["upper"]
        assert estimate == population["fixed_effects"][row["name"]]["value"]
        assert math.isclose(row["log_coordinate_standard_error"], se / estimate, rel_tol=1e-12)
        for bound, sign in (("lower", -1), ("upper", 1)):
            expected = math.exp(math.log(estimate) + sign * Z * se / estimate)
            assert math.isclose(row[bound], expected, rel_tol=1e-12), (row["name"], bound)
    index = [row["name"] for row in population["uncertainty"]["parameters"]].index
    for row in intervals["intervals"]:
        assert row["source_standard_error"] == population["uncertainty"]["standard_errors"][
            index(row["name"])]
    # Values stated in the tutorial prose.
    stated = {"theta_intercept": ("3.584", "3.172", "4.050", "4.055"),
              "theta_slope": ("1.955", "1.738", "2.199", "2.014")}
    for row in intervals["intervals"]:
        truth = design["truth"][row["name"]]
        values = tuple(f"{v:.3f}" for v in (row["estimate"], row["lower"], row["upper"], truth))
        assert values == stated[row["name"]], (row["name"], values)
        assert 0.055 < row["log_coordinate_standard_error"] < 0.065
        for bound, sign in (("lower", -1), ("upper", 1)):
            natural = row["estimate"] + sign * Z * row["source_standard_error"]
            assert abs(natural / row[bound] - 1) < 0.01
    by_name = {row["name"]: row for row in intervals["intervals"]}
    assert by_name["theta_intercept"]["upper"] < design["truth"]["theta_intercept"]
    assert (by_name["theta_slope"]["lower"] < design["truth"]["theta_slope"]
            < by_name["theta_slope"]["upper"])
    assert design["subjects"] == 32 and design["observations_per_subject"] == 6
    assert len(observed) == 192 == len(population["fitted_observations"])
    log_theta = [math.log(population["fixed_effects"][name]["value"]) for name in NAMES]
    rows = []
    for record in population["fitted_observations"]:
        source = observed[(observed["subject_index"] == record["subject_index"])
                          & (observed["row"] == record["row"])]
        assert len(source) == 1
        time = record["time"]["value"]
        assert math.isclose(time, source["time_h"].iloc[0], abs_tol=1e-12)
        assert math.isclose(record["value"]["value"], source["z_dimensionless"].iloc[0],
                            rel_tol=1e-12)
        eta = population["subjects"][record["subject_index"]]["eta"]
        pred = log_theta[0] + (time - 1) * log_theta[1]
        ipred = log_theta[0] + eta[0] + (time - 1) * (log_theta[1] + eta[1])
        assert math.isclose(record["pred"]["value"], pred, rel_tol=1e-10, abs_tol=1e-12)
        assert math.isclose(record["ipred"]["value"], ipred, rel_tol=1e-10, abs_tol=1e-12)
        rows.append({"subject": record["subject_index"], "time": time,
                     "observed": record["value"]["value"], "pred": pred,
                     "ipred": record["ipred"]["value"]})
    return population, design, pd.DataFrame(rows)


def intervals_against_truth(population, design):
    """Figure 1: estimates, log-scale Wald intervals, true values and individual values."""
    records = population["fixed_effect_intervals"]["intervals"]
    summary = pd.DataFrame([{"parameter": LABELS[row["name"]], "estimate": row["estimate"],
                             "lower": row["lower"], "upper": row["upper"],
                             "truth": design["truth"][row["name"]], "y": 0.0}
                            for row in records])
    individual = pd.DataFrame([{"parameter": LABELS[name],
                                "value": population["fixed_effects"][name]["value"]
                                * math.exp(subject["eta"][k]), "y": 1.0}
                               for subject in population["subjects"]
                               for k, name in enumerate(NAMES)])
    order = list(LABELS.values())
    for frame in (summary, individual):
        frame["parameter"] = pd.Categorical(frame["parameter"], order)
    left = individual.groupby("parameter", observed=True)["value"].min()
    notes = summary.assign(x=[left[p] for p in summary["parameter"]], y=-0.3,
                           label=[f"95% interval {lo:.3f} to {up:.3f}; true value {t:.3f}"
                                  for lo, up, t in zip(summary["lower"], summary["upper"],
                                                       summary["truth"])])
    rows = {0.0: "Fixed effect", 1.0: "Individual values"}
    plot = (ggplot()
            + geom_vline(summary, aes(xintercept="truth", linetype="'True value'"), color=INK,
                         size=0.6)
            + geom_jitter(individual, aes("value", "y", color="'Individual value (32 subjects)'"),
                          position=position_jitter(width=0, height=0.12, random_state=7),
                          size=1.8, alpha=0.75)
            + geom_segment(summary, aes(x="lower", xend="upper", y="y", yend="y",
                                        color="'Estimate with 95% log-scale Wald interval'"),
                           size=0.9)
            + geom_point(summary, aes("estimate", "y",
                                      color="'Estimate with 95% log-scale Wald interval'"),
                         size=3.2)
            + geom_text(notes, aes("x", "y", label="label"), ha="left", va="center",
                        color=INK, size=8.5)
            + facet_wrap("~parameter", ncol=1, scales="free_x")
            + scale_x_log10(breaks=[1, 1.5, 2, 2.5, 3, 4, 5, 6])
            + scale_y_continuous(breaks=list(rows), labels=list(rows.values()), limits=(-0.5, 1.4))
            + scale_color_manual(values=[CATEGORICAL[1], CATEGORICAL[0]])
            + scale_linetype_manual(values=["dashed"])
            + labs(x="Parameter value (dimensionless, log scale)", y="")
            + style(width=7.0, height=4.4)
            + theme(legend_direction="vertical", legend_box="horizontal"))
    return save(plot, "population-log-wald-intervals")


def interval_shape(population):
    """Figure 2: analytical interval endpoints as a function of the relative standard error."""
    rse = np.linspace(0.0, 0.8, 161)
    curves = pd.concat([
        pd.DataFrame({"rse": rse, "lower": np.exp(-Z * rse), "upper": np.exp(Z * rse),
                      "method": "Log-scale Wald (reported)"}),
        pd.DataFrame({"rse": rse, "lower": 1 - Z * rse, "upper": 1 + Z * rse,
                      "method": "Natural-scale Wald (for comparison)"}),
    ])
    order = ["Log-scale Wald (reported)", "Natural-scale Wald (for comparison)"]
    curves["method"] = pd.Categorical(curves["method"], order)
    long = pd.concat([curves.assign(bound="lower", ratio=curves["lower"]),
                      curves.assign(bound="upper", ratio=curves["upper"])])
    long["group"] = long["method"].astype(str) + long["bound"]
    fitted = pd.DataFrame([{"rse": row["log_coordinate_standard_error"],
                            "ratio": row[bound] / row["estimate"], "name": LABELS[row["name"]]}
                           for row in population["fixed_effect_intervals"]["intervals"]
                           for bound in ("lower", "upper")])
    crossing = 1 / Z
    plot = (ggplot()
            + geom_ribbon(curves[curves["method"] == order[0]],
                          aes("rse", ymin="lower", ymax="upper"), fill=FILL_LIGHT, alpha=0.25)
            + geom_hline(yintercept=0, color=MUTED, size=0.4)
            + geom_hline(yintercept=1, color=MUTED, size=0.4, linetype="dotted")
            + geom_line(long, aes("rse", "ratio", color="method", linetype="method",
                                  group="group"), size=0.8)
            + geom_point(fitted, aes("rse", "ratio"), color=INK, size=2.4)
            + annotate("text", x=0.09, y=1.55, label="this fit", ha="left", color=INK, size=9)
            + annotate("point", x=crossing, y=0, color=CATEGORICAL[1], size=2.4)
            + annotate("text", x=crossing - 0.02, y=-0.28,
                       label=f"natural-scale lower limit reaches 0 at SE/estimate = {crossing:.2f}",
                       ha="right", color=CATEGORICAL[1], size=9)
            + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[1]])
            + scale_linetype_manual(values=["solid", "dashed"])
            + scale_x_continuous(breaks=[0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8])
            + scale_y_continuous(breaks=[-0.5, 0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5, 4, 4.5],
                                 limits=(-0.6, 4.8))
            + labs(x="Relative standard error, SE/estimate",
                   y="Interval endpoint / estimate")
            + style(width=7.0, height=4.4))
    return save(plot, "population-log-wald-interval-shape")


def fitted_predictions(design, data):
    """Figure 3: observations with population predictions, and observed against IPRED."""
    truth = [math.log(design["truth"][name]) for name in NAMES]
    grid = np.linspace(0, 2, 41)
    estimated = data.drop_duplicates("time").sort_values("time")
    kinds = ["Individual prediction", "Population prediction", "True typical value"]
    lines = pd.concat([
        data.assign(value=data["ipred"], kind=kinds[0], group=data["subject"].astype(str)),
        estimated.assign(value=estimated["pred"], kind=kinds[1], group="pred"),
        pd.DataFrame({"time": grid, "value": truth[0] + (grid - 1) * truth[1],
                      "kind": kinds[2], "group": "truth"}),
    ])
    lines["kind"] = pd.Categorical(lines["kind"], kinds)
    over_time = (ggplot()
                 + geom_line(lines, aes("time", "value", group="group", color="kind",
                                        linetype="kind", size="kind", alpha="kind"))
                 + geom_point(data, aes("time", "observed"), color=INK, fill="white", shape="o",
                              size=1.6, stroke=0.6, alpha=0.8)
                 + scale_color_manual(values=[FILL_LIGHT, CATEGORICAL[1], INK])
                 + scale_linetype_manual(values=["solid", "solid", "dashed"])
                 + scale_size_manual(values=[0.4, 1.1, 0.7])
                 + scale_alpha_manual(values=[0.7, 1, 1])
                 + guides(color=guide_legend(ncol=1))
                 + labs(x="Time (h)", y="Response z (dimensionless)",
                        title="A  Observations and predictions")
                 + style(width=3.9, height=4.6))
    against = (ggplot(data, aes("ipred", "observed"))
               + geom_abline(intercept=0, slope=1, color=MUTED, linetype="dashed", size=0.5)
               + geom_point(color=INK, fill="white", shape="o", size=1.6, stroke=0.6, alpha=0.8)
               + labs(x="Individual prediction", y="Observed response",
                      title="B  Observed against individual prediction")
               + style(width=3.6, height=4.6))
    plot = Beside([over_time, against]) + theme(figure_size=(8.0, 4.8))
    return save(plot, "population-log-wald-fit")


def main():
    population, design, data = checked()
    for path in (intervals_against_truth(population, design), interval_shape(population),
                 fitted_predictions(design, data)):
        print(path.name)


if __name__ == "__main__":
    main()
