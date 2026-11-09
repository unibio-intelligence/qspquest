"""Figure for the first-simulation tutorial (first-run-journeys.md).

Runs the Python route (``python -m qspquest.first_run``) and the CLI route
(``scientist_first_run.py``) into a temporary directory, checks the saved tables
against the values printed in the tutorial and against the analytical solution
``cp(t) = 2 exp(-0.1 t)``, and writes one PNG figure to docs/tutorials/figures/.
Requires the ``docs`` extra (pandas and plotnine) and a built CLI.
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
    aes, facet_wrap, geom_blank, geom_hline, geom_line, geom_point, geom_text, ggplot, labs,
    scale_color_manual, scale_shape_manual, scale_x_continuous, scale_y_log10, theme,
)

from figure_style import CATEGORICAL, INK, save, style

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
TOLERANCE = 1e-8  # mg/L, the limit applied by both helpers
DOCUMENTED = {0: 2.000000, 2: 1.637462, 8: 0.898658, 24: 0.181436}  # Results table
ROUTES = {"Python package": "python", "Command-line interface": "cli"}


def analytical(t):
    return 2.0 * math.exp(-0.1 * t)


def run_routes(out, cli):
    """Run both helpers exactly as the tutorial does; return {route: directory}."""
    python_dir, cli_dir = out / "developer-results", out / "scientist-results"
    subprocess.run([sys.executable, "-m", "qspquest.first_run", "--out", str(python_dir)],
                   check=True, capture_output=True)
    subprocess.run([sys.executable, str(HERE / "scientist_first_run.py"), "--binary", str(cli),
                    "--model", str(ROOT / "conformance/models/synthetic-one-compartment.qsp"),
                    "--request", str(HERE / "first-run.json"), "--out", str(cli_dir)],
                   check=True, capture_output=True)
    return {"Python package": python_dir, "Command-line interface": cli_dir}


def read_route(route, directory):
    with (directory / "first-run.csv").open() as stream:
        rows = list(csv.DictReader(stream))
    checks = json.loads((directory / "checks.json").read_text())
    manifest = json.loads((directory / "first-run.csv.manifest.json").read_text())
    assert checks["status"] == "passed" and checks["observations"] == 4
    assert checks["max_absolute_error_mg_L"] < TOLERANCE
    assert manifest["producer"] == ROUTES[route]
    frame = pd.DataFrame({"time": [float(r["time_h"]) for r in rows],
                          "conc": [float(r["cp_mg_per_L"]) for r in rows], "route": route})
    assert list(frame["time"]) == list(DOCUMENTED)
    for t, c in zip(frame["time"], frame["conc"]):
        assert round(c, 6) == DOCUMENTED[int(t)], (route, t, c)
    frame["error"] = [abs(c - analytical(t)) for t, c in zip(frame["time"], frame["conc"])]
    assert frame["error"].max() < 1e-9  # stated in the Results section
    assert abs(frame["error"].max() - checks["max_absolute_error_mg_L"]) < 1e-15
    return frame


def figure(points):
    """Concentration and its difference from the analytical solution, side by side."""
    panels = ["A  Concentration, cp (mg/L)", "B  |simulated − analytical| (mg/L)"]
    points["route"] = pd.Categorical(points["route"], list(ROUTES))
    dense = pd.DataFrame({"time": np.linspace(0, 24, 241)})
    dense["value"] = [analytical(t) for t in dense["time"]]
    dense["panel"] = panels[0]
    values = pd.concat([points.assign(value=points["conc"], panel=panels[0]),
                        points[points["error"] > 0].assign(value=lambda f: f["error"],
                                                           panel=panels[1])])
    limit = pd.DataFrame({"value": [TOLERANCE], "panel": [panels[1]]})
    label = pd.DataFrame({"time": [23.5], "value": [TOLERANCE * 1.6], "panel": [panels[1]],
                          "text": ["Acceptance limit, 1e-8 mg/L"]})
    # Invisible points that fix the range of panel B.
    span = pd.DataFrame({"time": [0, 0], "value": [1e-12, 1e-7], "panel": panels[1]})
    for frame in (dense, values, limit, label, span):
        frame["panel"] = pd.Categorical(frame["panel"], panels)
    plot = (ggplot()
            + geom_blank(span, aes("time", "value"))
            + geom_line(dense, aes("time", "value"), color=INK, size=0.7)
            + geom_hline(limit, aes(yintercept="value"), color=INK, linetype="dashed", size=0.6)
            + geom_text(label, aes("time", "value", label="text"), ha="right", va="bottom",
                        color=INK, size=8.5)
            + geom_point(values, aes("time", "value", color="route", shape="route"),
                         size=3.2, stroke=1.0, fill="none")
            + facet_wrap("~panel", scales="free_y")
            + scale_y_log10(breaks=[0.2, 0.5, 1, 2] + [10.0 ** e for e in range(-12, -6)],
                            labels=lambda breaks: [f"{b:g}" if b > 1e-3 else f"1e{math.log10(b):.0f}"
                                                   for b in breaks])
            + scale_x_continuous(breaks=[0, 2, 8, 12, 24])
            + scale_color_manual(values=CATEGORICAL[:2])
            + scale_shape_manual(values=["o", "x"])
            + labs(x="Time after dose (h)", y="Log scale")
            + style(width=8.4, height=4.2)
            + theme(panel_spacing=0.06))
    return save(plot, "first-run-journeys-check")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/release/qspquest")
    args = parser.parse_args()
    if not args.cli.is_file():
        parser.error("Build the CLI first (cargo build --release --locked -p qspquest-cli) "
                     "or pass --cli: " + str(args.cli))
    with tempfile.TemporaryDirectory() as scratch:
        directories = run_routes(Path(scratch), args.cli.resolve())
        frames = [read_route(route, path) for route, path in directories.items()]
    python, cli = frames
    assert max(abs(python["conc"] - cli["conc"])) < 1e-12, "Python and CLI tables differ"
    print(figure(pd.concat(frames, ignore_index=True)).name)


if __name__ == "__main__":
    main()
