"""Figures for the library fitting tutorial.

Runs mcp_library_fit.py for the three synthetic workflows of the normalized
scalar example in a temporary directory, checks the saved results, simulates the
archived native model with the Python binding at the starting values and the
estimates, compares the simulation with the closed-form solution for repeated
zero-order infusions, and writes one PNG figure to docs/tutorials/figures/.

Requires a built MCP server (cargo build --release --locked -p qspquest-mcp)
and the ``docs`` extra (pandas and plotnine).
"""

import argparse
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, element_rect, geom_abline, geom_line, geom_point, geom_rect, ggplot, labs,
    scale_color_manual, scale_linetype_manual, scale_shape_manual, scale_x_continuous,
    scale_y_continuous, theme,
)
from plotnine.composition import Beside

import qspquest as qq
from figure_style import CATEGORICAL, FILL_LIGHT, INK, save, style

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
LIBRARY = ROOT / "bindings/mcp/tests/library"
MODEL, VERSION = "qspquest_diffdose_2026_idr_benchmark", "0.3.1"
WORKFLOWS = ["diffsol_bdf-start-1", "diffsol_bdf-start-2", "diffsol_bdf-start-3"]
PARAMETER, OUTPUT, VOLUME_PARAMETER = "source_symbol_5", "source_output_0", "source_symbol_4"
TRUTH, LIMIT = 0.7, 1e-5
TOLERANCE = 1e-8  # between the BDF solution and the closed form, in unit 1


def run_client(binary, directory):
    """Run the tutorial client once per workflow and read the saved files."""
    runs = {}
    for workflow in WORKFLOWS:
        out = directory / workflow
        subprocess.run([sys.executable, str(HERE / "mcp_library_fit.py"), str(binary),
                        "--library", str(LIBRARY), "--model", MODEL, "--version", VERSION,
                        "--workflow", workflow, "--out", str(out)],
                       check=True, cwd=HERE, stdout=subprocess.DEVNULL)
        files = {name: json.loads((out / f"{name}.json").read_text())
                 for name in ("checks", "result", "arguments")}
        check = files["checks"]
        assert check["status"] == "passed" and check["fit_status"] == "converged"
        assert check["parameter_checks"][PARAMETER]["passed"] is True
        estimate = files["result"]["parameters"][PARAMETER]
        assert estimate["unit"] == "1" and abs(estimate["value"] - TRUTH) <= LIMIT
        runs[workflow] = files
    return runs


def native_model(result):
    archive = LIBRARY / result["model_provenance"]["archive"]
    with zipfile.ZipFile(archive) as bundle:
        model = qq.CompiledModel(json.loads(bundle.read("native/model.json")))
    assert model.model_content_hash == result["identity"]["model_content_hash"]
    return model


def simulate(model, run, value, times=None):
    run = json.loads(json.dumps(run))
    run["parameters"][PARAMETER] = {"unit": "1", "value": value}
    if times is not None:
        run["regimen"]["observations"] = [{"time": {"unit": "d", "value": float(t)}, "side": "post"}
                                          for t in times]
    result = model.run(run)
    (output,) = [o for o in result["outputs"] if o["name"] == OUTPUT]
    return np.asarray(result["times"]), np.asarray(output["values"])


def closed_form(run, k, t):
    """Superposition of zero-order infusions into a one-compartment model, C = A/V."""
    volume = run["parameters"][VOLUME_PARAMETER]["value"]
    amount = 0.0
    for dose in run["regimen"]["administrations"]:
        start = dose["time"]["value"]
        duration = dose["delivery"]["span"]["duration"]["value"]
        rate = dose["amount"]["value"] / duration
        if t <= start:
            continue
        if t <= start + duration:
            amount += rate / k * (1 - math.exp(-k * (t - start)))
        else:
            amount += rate / k * (1 - math.exp(-k * duration)) * math.exp(-k * (t - start - duration))
    return amount / volume


def figure(runs):
    first = runs[WORKFLOWS[0]]["result"]
    model = native_model(first)
    run = first["effective_request"]["run"]
    observations = first["effective_request"]["observations"]
    times = [o["time"]["value"] for o in run["regimen"]["observations"]]
    observed = pd.DataFrame({"time": [times[o["row"]] for o in observations],
                             "conc": [o["value"]["value"] for o in observations]})
    assert all(o["output"] == OUTPUT for o in observations) and len(observed) == 15
    dense = np.round(np.linspace(0, 8, 801), 6)
    curves, pairs = [], []
    for index, workflow in enumerate(WORKFLOWS):
        files = runs[workflow]
        request = files["result"]["effective_request"]
        assert request["observations"] == observations
        start = request["run"]["parameters"][PARAMETER]["value"]
        estimate = files["result"]["parameters"][PARAMETER]["value"]
        for label, value in ((f"Starting value {start:g}", start), ("Estimate", estimate)):
            t, c = simulate(model, request["run"], value)
            assert max(abs(c - [closed_form(request["run"], value, x) for x in t])) < TOLERANCE
            pairs.append(pd.DataFrame({"observed": observed["conc"], "predicted": c,
                                       "kind": "Estimate (three fits)" if label == "Estimate"
                                       else label}))
            if label == "Estimate":
                sd = observations[0]["error"]["additive_sd"]["value"]
                nll = float(np.sum((observed["conc"] - c) ** 2) / (2 * sd ** 2)
                            + len(c) * (math.log(sd) + 0.5 * math.log(2 * math.pi)))
                assert abs(nll - files["result"]["objective"]["negative_log_likelihood"]) < 1e-6
                assert max(abs(observed["conc"] - c)) < 1e-6
            if label != "Estimate" or index == 0:
                t, c = simulate(model, request["run"], value, dense)
                curves.append(pd.DataFrame({"time": t, "conc": c,
                                            "kind": "Estimate (three fits)" if label == "Estimate"
                                            else label}))
    order = [f"Starting value {s:g}" for s in (0.3, 0.5, 1.0)] + ["Estimate (three fits)"]
    curves, pairs = pd.concat(curves), pd.concat(pairs)
    assert set(curves["kind"]) == set(order)
    for frame in (curves, pairs):
        frame["kind"] = pd.Categorical(frame["kind"], order)
    colors = dict(zip(order, CATEGORICAL + [INK]))
    infusions = pd.DataFrame([{"xmin": d["time"]["value"],
                               "xmax": d["time"]["value"] + d["delivery"]["span"]["duration"]["value"]}
                              for d in run["regimen"]["administrations"]])
    top = 1.05 * max(curves["conc"].max(), pairs["predicted"].max())
    time_course = (ggplot()
                   + geom_rect(infusions, aes(xmin="xmin", xmax="xmax"), ymin=0, ymax=top,
                               fill=FILL_LIGHT, alpha=0.25)
                   + geom_line(curves, aes("time", "conc", color="kind",
                                           linetype="kind"), size=0.75)
                   + geom_point(observed, aes("time", "conc"), color=INK, fill="white", shape="o",
                                size=2.3, stroke=0.9)
                   + annotate("text", x=0.15, y=top * 0.985, label="Infusions", ha="left",
                              va="top", color=INK, size=8)
                   + scale_color_manual(values=colors)
                   + scale_linetype_manual(values={k: "dashed" for k in order[:3]}
                                           | {order[3]: "solid"})
                   + scale_x_continuous(breaks=range(0, 9))
                   + scale_y_continuous(limits=(0, top), expand=(0, 0))
                   + labs(x="Time (days)", y="Concentration C (normalized, unit 1)",
                          title="A  Concentration–time course")
                   + style(width=4.4, height=4.2)
                   + theme(legend_position="none"))
    limit = (0, 9)
    assert pairs["predicted"].max() < 9 and pairs["observed"].max() < 9
    agreement = (ggplot(pairs, aes("predicted", "observed", color="kind", shape="kind"))
                 + geom_abline(intercept=0, slope=1, color=INK, linetype="dotted", size=0.5)
                 + geom_point(size=2.2, fill="white", stroke=0.9)
                 + scale_color_manual(values=colors)
                 + scale_shape_manual(values={order[0]: "o", order[1]: "s", order[2]: "^",
                                              order[3]: "D"})
                 + scale_x_continuous(limits=limit, breaks=range(0, 10, 3))
                 + scale_y_continuous(limits=limit, breaks=range(0, 10, 3))
                 + labs(x="Predicted C (unit 1)", y="Observed C (unit 1)",
                        title="B  Observed against predicted")
                 + style(width=4.4, height=4.2)
                 + theme(legend_position=(0.03, 0.97), legend_justification=(0, 1),
                         legend_background=element_rect(fill="white", color="white")))
    plot = Beside([time_course, agreement]) + theme(figure_size=(9.0, 4.4))
    return save(plot, "library-fitting-observed-predicted")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("binary", type=Path, nargs="?", default=ROOT / "target/release/qspquest-mcp")
    args = parser.parse_args()
    if not args.binary.exists():
        parser.error(f"{args.binary} not found; build it with cargo build --release --locked -p qspquest-mcp")
    with tempfile.TemporaryDirectory() as directory:
        runs = run_client(args.binary.resolve(), Path(directory))
        print(figure(runs).name)


if __name__ == "__main__":
    main()
