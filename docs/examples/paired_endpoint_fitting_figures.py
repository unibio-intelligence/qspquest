"""Figures for the paired-endpoint fitting tutorial.

Runs the ``prepare`` and ``run`` stages of paired_endpoint_fit.py in a temporary
directory, checks the saved fits against the synthetic truth and the independent
Runge-Kutta reference, and writes two PNG figures to docs/tutorials/figures/.
Requires the ``interchange`` and ``docs`` extras (libSBML, pandas and plotnine).
"""

from copy import deepcopy
import json
import math
from pathlib import Path
import tempfile

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, element_text, geom_line, geom_point, geom_raster, geom_segment,
    ggplot, labs, scale_fill_gradientn, scale_shape_manual, scale_x_continuous,
    scale_y_continuous, scale_y_log10, theme,
)
from plotnine.composition import Stack

import qspquest as qq
from figure_style import CATEGORICAL, INK, SEQUENTIAL, save, style
import paired_endpoint_fit as paired

ENDPOINTS = {"measured_concentration": "Concentration (mg/L, log scale)",
             "measured_cells": "Cell density (1/mm³)"}
TIME_BREAKS = [0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330, 360]


def load(directory):
    """Prepare and run the tutorial example, then read the saved outputs."""
    prepared_receipt = paired.prepare(directory / "input")
    report = paired.run(directory / "input" / "prepared.json",
                        prepared_receipt["prepared_sha256"], directory / "results")
    assert report["passed"] is True
    assert report["registered_fit_denominator"] == 3 and len(report["cases"]) == 3
    assert report["registered_control_denominator"] == 5 and len(report["controls"]) == 5
    prepared = json.loads((directory / "input" / "prepared.json").read_bytes())
    fits = {}
    for case in report["cases"]:
        name = case["case"]
        result = json.loads((directory / "results" / f"{name}-result.json").read_bytes())
        diagnostic = json.loads((directory / "results" / f"{name}-diagnostics.json").read_bytes())
        fit = result["fit"]
        assert fit["status"] == "converged" and diagnostic["parameter_recovery"] is True
        for parameter, truth in paired.TRUTH.items():
            limit = prepared["recovery_absolute_limits"][parameter]
            assert abs(fit["parameters"][parameter]["value"] - truth) <= limit, (name, parameter)
        assert len(diagnostic["observations"]) == 40
        fits[name] = {"fit": fit, "diagnostic": diagnostic,
                      "request": prepared["requests"][name]}
    return prepared, fits


def dense_run(model, request, parameters):
    """Simulate the fitted model on a dense grid, with pre- and post-dose rows at day 180."""
    run = deepcopy(request["problem"]["request"]["objective"]["simulation"]["run"])
    run["parameters"].update(parameters)
    dense = {float(t) for t in np.linspace(0, 360, 721)} | {t for t, _ in paired.GRID}
    grid = [(t, "post") for t in sorted(dense - {0.0, 180.0})]
    grid = [(0.0, "pre"), (0.0, "post")] + grid
    grid.insert(next(i for i, (t, _) in enumerate(grid) if t > 180), (180.0, "post"))
    grid.insert(next(i for i, (t, _) in enumerate(grid) if t == 180), (180.0, "pre"))
    run["regimen"]["observations"] = [{"time": paired.quantity(t, "d"), "side": side}
                                      for t, side in grid]
    run["regimen"]["samples"] = []
    result = model.run(run)
    frame = pd.DataFrame({"time": result["times"], "side": result["sides"]})
    for output in result["outputs"]:
        frame[output["name"]] = output["values"]
    return frame


def profiles(prepared, fits):
    """Figure 1: both endpoints, observed and fitted, on a shared time axis."""
    name = sorted(fits)[0]
    fit = fits[name]["fit"]
    model = qq.CompiledModel(prepared["model"])
    curve = dense_run(model, fits[name]["request"], fit["parameters"])
    # The fitted curve passes through the native predictions and the independent RK4
    # solution at the fitted parameters.
    independent = paired.reference({p: fit["parameters"][p]["value"] for p in paired.TRUTH},
                                   paired.REFERENCE_STEPS[1])
    for row in fits[name]["diagnostic"]["observations"]:
        time, side = row["observation"]["time"]["value"], row["observation"]["side"]
        match = curve[(curve["time"] == time) & (curve["side"] == side)]
        assert len(match) == 1, (time, side)
        simulated = float(match[row["endpoint"]].iloc[0])
        assert abs(simulated - row["predicted"]) <= 1e-6 + 1e-5 * abs(row["predicted"])
        column = list(paired.SD).index(row["endpoint"])
        expected = independent[row["result_row"]][column]
        assert abs(row["predicted"] - expected) <= 1e-6 + 1e-5 * abs(expected)
        assert abs(row["standardized_residual"]) < 0.01
    observed = pd.DataFrame([{"time": r["observation"]["time"]["value"], "endpoint": r["endpoint"],
                              "value": r["observed"], "side": r["observation"]["side"]}
                             for r in fits[name]["diagnostic"]["observations"]])
    assert len(observed) == 40 and set(observed["endpoint"]) == set(ENDPOINTS)
    dose = pd.DataFrame({"time": [0, 180]})
    panels = []
    for index, (endpoint, title) in enumerate(ENDPOINTS.items()):
        points = observed[observed["endpoint"] == endpoint]
        line = curve[["time", endpoint]].rename(columns={endpoint: "value"})
        if endpoint == "measured_concentration":
            points, line = points[points["value"] > 0], line[line["value"] > 0]
        plot = (ggplot()
                + geom_line(line, aes("time", "value"), color=CATEGORICAL[index], size=0.8)
                + geom_point(points, aes("time", "value"), color=CATEGORICAL[index], size=2.2,
                             fill="white", stroke=0.9, shape="o")
                + scale_x_continuous(breaks=TIME_BREAKS, limits=(0, 360))
                + labs(x="Time (days)" if index else "", y=title,
                       title="A  Antibody concentration" if not index else "B  Cell density")
                + style(width=7.0, height=3.0))
        if endpoint == "measured_concentration":
            plot = (plot + scale_y_log10(limits=(0.008, 1500),
                                         breaks=[0.01, 0.1, 1, 10, 100, 1000],
                                         labels=["0.01", "0.1", "1", "10", "100", "1000"])
                    + annotate("text", x=dose["time"] + 3, y=[900, 900],
                               label=["1000 mg bolus", "500 mg bolus"], ha="left", va="center",
                               color=INK, size=8))
        else:
            plot = plot + scale_y_continuous(limits=(0, 205))
        panels.append(plot)
    plot = Stack(panels) + theme(figure_size=(7.0, 6.2))
    return save(plot, "paired-endpoint-fitting-profiles")


def surface(prepared, fits):
    """Figure 2: objective surface with the three starting points and estimates."""
    name = sorted(fits)[0]
    objective = deepcopy(fits[name]["request"]["problem"]["request"]["objective"])
    sensitivities = qq.CompiledSensitivities(prepared["model"], list(paired.TRUTH))
    (cl_low, cl_high), (effect_low, effect_high) = paired.BOUNDS.values()
    cl_grid = np.geomspace(cl_low, cl_high, 41)
    effect_grid = np.geomspace(effect_low, effect_high, 41)
    rows = []
    for cl in cl_grid:
        for effect in effect_grid:
            trial = deepcopy(objective)
            trial["simulation"]["run"]["parameters"]["cl"]["value"] = float(cl)
            trial["simulation"]["run"]["parameters"]["drug_effect"]["value"] = float(effect)
            rows.append({"cl": cl, "effect": effect,
                         "objective": sensitivities.objective(trial)["objective"]})
    grid = pd.DataFrame(rows)
    minimum = min(fit["fit"]["objective"]["objective"] for fit in fits.values())
    assert grid["objective"].min() >= minimum - 1e-6
    best = grid.loc[grid["objective"].idxmin()]
    assert abs(math.log(best["cl"] / paired.TRUTH["cl"])) < 0.06
    assert abs(math.log(best["effect"] / paired.TRUTH["drug_effect"])) < 0.06
    # Independent check of the native objective at the truth: the RK4 reference
    # reproduces the observations, so the objective equals the normalizing constant.
    trial = deepcopy(objective)
    for parameter, truth in paired.TRUTH.items():
        trial["simulation"]["run"]["parameters"][parameter]["value"] = truth
    constant = sum(math.log(o["error"]["additive_sd"]["value"]) + 0.5 * math.log(2 * math.pi)
                   for o in objective["observations"])
    assert abs(paired.reference_objective(trial) - constant) < 1e-3
    assert abs(minimum - constant) < 1e-3
    grid["log_delta"] = np.log10(grid["objective"] - minimum + 1)
    grid["x"], grid["y"] = np.log10(grid["cl"]), np.log10(grid["effect"])
    paths = []
    for index, (case, entry) in enumerate(sorted(fits.items())):
        start = entry["request"]["problem"]["request"]["objective"]["simulation"]["run"]["parameters"]
        estimate = entry["fit"]["parameters"]
        assert start["cl"]["value"] == paired.STARTS[index][0]
        paths.append({"x0": math.log10(start["cl"]["value"]),
                      "y0": math.log10(start["drug_effect"]["value"]),
                      "x1": math.log10(estimate["cl"]["value"]),
                      "y1": math.log10(estimate["drug_effect"]["value"])})
    paths = pd.DataFrame(paths)
    markers = pd.concat([
        pd.DataFrame({"x": paths["x0"], "y": paths["y0"], "kind": "Starting value"}),
        pd.DataFrame({"x": paths["x1"], "y": paths["y1"], "kind": "Estimate (three fits)"}),
    ])
    cl_ticks, effect_ticks = [0.01, 0.02, 0.04, 0.1], [0.001, 0.002, 0.003, 0.006]
    plot = (ggplot()
            + geom_raster(grid, aes("x", "y", fill="log_delta"), interpolate=True)
            + geom_segment(paths, aes(x="x0", y="y0", xend="x1", yend="y1"), color="white",
                           linetype="dashed", size=0.6)
            + geom_point(markers, aes("x", "y", shape="kind"), color="white", fill=INK,
                         size=3.2, stroke=0.9)
            + scale_shape_manual(values={"Starting value": "o", "Estimate (three fits)": "D"})
            + scale_fill_gradientn(colors=list(reversed(SEQUENTIAL)), name="NLL − minimum",
                                   breaks=[3, 4, 5, 6, 7],
                                   labels=["10³", "10⁴", "10⁵", "10⁶", "10⁷"])
            + scale_x_continuous(breaks=[math.log10(t) for t in cl_ticks],
                                 labels=[f"{t:g}" for t in cl_ticks], expand=(0, 0))
            + scale_y_continuous(breaks=[math.log10(t) for t in effect_ticks],
                                 labels=[f"{t:g}" for t in effect_ticks], expand=(0, 0))
            + labs(x="cl (1/day, log scale)", y="drug_effect (L/(mg·day), log scale)", shape="")
            + style(width=6.4, height=5.0)
            + theme(legend_position="right", legend_title=element_text(color=INK, size=9)))
    return save(plot, "paired-endpoint-fitting-objective")


def main():
    with tempfile.TemporaryDirectory() as directory:
        prepared, fits = load(Path(directory))
        for figure in (profiles, surface):
            print(figure(prepared, fits).name)


if __name__ == "__main__":
    main()
