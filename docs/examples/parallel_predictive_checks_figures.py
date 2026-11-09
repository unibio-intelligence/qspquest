"""Figure for the parallel predictive checks tutorial.

Runs parallel_vpc.py with one and with two workers in a temporary directory,
checks the saved results against the values stated in the tutorial, and writes
one PNG figure to docs/tutorials/figures/. Requires the ``docs`` extra (pandas
and plotnine).
"""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
import pandas as pd
from plotnine import (
    aes, facet_wrap, geom_crossbar, geom_point, geom_text, geom_vline, ggplot, guide_legend,
    guides, labs, scale_fill_manual, scale_shape_manual, scale_x_continuous,
    scale_y_log10, theme,
)
from plotnine.composition import Stack

from figure_style import CATEGORICAL, FILL_LIGHT, INK, MUTED, save, style
from parallel_vpc import synthetic_cohort

SCRIPT = Path(__file__).resolve().parent / "parallel_vpc.py"
TIMES = [0.5, 2, 8, 24]
EDGES = [0, 2, 8, 24]
PERCENTILES = {0.05: "5th", 0.5: "50th", 0.95: "95th"}


def run_example(directory, workers):
    output = directory / f"vpc-{workers}.json"
    subprocess.run([sys.executable, str(SCRIPT), "--workers", str(workers), "--output", str(output)],
                   check=True, capture_output=True, text=True)
    return json.loads(output.read_text())


def observations():
    _, subjects = synthetic_cohort()
    rows = [{"subject": subject["id"], "time": TIMES[row["row"]], "conc": row["value"]["value"]}
            for subject in subjects for row in subject["observations"]]
    return pd.DataFrame(rows)


def checked_result():
    with tempfile.TemporaryDirectory(prefix="qspquest-vpc-figures-") as temporary:
        serial = run_example(Path(temporary), 1)
        parallel = run_example(Path(temporary), 2)
    for result in (serial, parallel):
        assert result["status"] == "complete"
        assert result["successful_replicates"] == 16
        assert result["failed_replicates"] == 0
        assert result["simulations"] == 102
        assert result["confidence"] == 0.95
    for key in ("bins", "replicate_records", "identity", "seed", "simulations"):
        assert serial[key] == parallel[key], key
    assert serial["execution"]["workers"] == 1 and parallel["execution"]["workers"] == 2
    observed = observations()
    assert len(observed) == serial["total_observations"] == 24
    bins = serial["bins"]
    assert [(b["start"], b["end"]) for b in bins] == list(zip(EDGES[:-1], EDGES[1:]))
    assert [b["observations"] for b in bins] == [6, 6, 12]
    for index, record in enumerate(bins):
        last = index == len(bins) - 1
        inside = observed[(observed["time"] >= record["start"])
                          & ((observed["time"] <= record["end"]) if last
                             else (observed["time"] < record["end"]))]
        assert len(inside) == record["observations"]
        for quantile in record["quantiles"]:
            expected = np.quantile(inside["conc"], quantile["percentile"])
            assert abs(quantile["observed"] - expected) < 1e-12
            low, high = quantile["confidence_interval"]
            assert 0 < low <= quantile["simulated"] <= high
            assert low <= quantile["observed"] <= high  # stated in the tutorial
            assert quantile["quantifiable_replicates"] == 16
    return serial, observed


def bin_label(record):
    return f"{record['start']:g}–{record['end']:g} h"


def predictive_check(result, observed):
    """Figure 1: observations by time, and observed percentiles against simulated intervals."""
    bins = result["bins"]
    labels = [bin_label(record) for record in bins]
    middles = pd.DataFrame({"time": [(a + b) / 2 for a, b in zip(EDGES[:-1], EDGES[1:])],
                            "label": labels})
    data = (ggplot(observed, aes("time", "conc"))
            + geom_vline(xintercept=EDGES, color=MUTED, linetype="dotted", size=0.5)
            + geom_point(color=INK, fill="white", shape="o", size=2.2, stroke=0.9)
            + geom_text(middles, aes("time", 2.9, label="label"), color=INK, size=8.5)
            + scale_y_log10(breaks=[0.1, 0.2, 0.5, 1, 2], labels=["0.1", "0.2", "0.5", "1", "2"],
                            limits=(0.07, 3.2))
            + scale_x_continuous(breaks=EDGES, labels=[f"{t:g}" for t in EDGES])
            + labs(x="Time after dose (h)", y="Concentration (mg/L, log scale)",
                   title="A  Observations and time bins")
            + style(width=7.0, height=2.6))
    rows = []
    for record, label in zip(bins, labels):
        for quantile in record["quantiles"]:
            low, high = quantile["confidence_interval"]
            rows.append({"bin": f"{label}, {record['observations']} observations",
                         "percentile": PERCENTILES[quantile["percentile"]],
                         "low": low, "high": high, "simulated": quantile["simulated"],
                         "observed": quantile["observed"]})
    summary = pd.DataFrame(rows)
    summary["bin"] = pd.Categorical(summary["bin"], list(dict.fromkeys(summary["bin"])))
    summary["percentile"] = pd.Categorical(summary["percentile"], list(PERCENTILES.values()))
    check = (ggplot(summary, aes("percentile"))
             + geom_crossbar(aes(y="simulated", ymin="low", ymax="high",
                                 fill="'Simulated percentile: median and 95% interval'"),
                             color=CATEGORICAL[0], width=0.55, size=0.5, alpha=0.45)
             + geom_point(aes(y="observed", shape="'Observed percentile'"), color=INK, size=2.6)
             + facet_wrap("~bin", ncol=3, scales="free_y")
             + scale_fill_manual(values=[FILL_LIGHT])
             + scale_shape_manual(values=["o"])
             + labs(x="Percentile", y="Concentration (mg/L)",
                    title="B  Observed percentiles and simulated 95% intervals")
             + guides(fill=guide_legend(order=1), shape=guide_legend(order=2))
             + style(width=7.0, height=3.4))
    plot = Stack([data, check]) + theme(figure_size=(7.0, 6.4))
    return save(plot, "parallel-predictive-checks-vpc")


def main():
    result, observed = checked_result()
    print(predictive_check(result, observed).name)


if __name__ == "__main__":
    main()
