"""Portable SVG/HTML views of scientific analysis results, without network I/O.

Colours come from the figure style file shared with ``qspquest.plot``
(``plot/figure-style.json``, light mode); the report needs no plotting
packages.
"""

from html import escape
import json
import math
from pathlib import Path
from ._analysis import fail

_STYLE = json.loads((Path(__file__).parent / "plot" / "figure-style.json").read_text())
_COLOURS = _STYLE["modes"]["light"]
_DASHES = ("", "6 3", "2 3")  # series beyond the palette also differ in line pattern


def _series_style(index):
    palette = _COLOURS["categorical"]
    return palette[index % len(palette)], _DASHES[(index // len(palette)) % len(_DASHES)]


def _plot(title, series, xlabel, ylabel):
    points = [
        (float(x), float(y))
        for _, xy, *_ in series
        for x, y in xy
        if x is not None and y is not None and math.isfinite(x) and math.isfinite(y)
    ]
    if not points:
        return ""
    x0, x1 = min(x for x, _ in points), max(x for x, _ in points)
    y0, y1 = min(y for _, y in points), max(y for _, y in points)
    dx, dy = max(x1 - x0, 1e-12), max(y1 - y0, 1e-12)
    content, legends = [], []
    lines = 0
    for i, (name, xy, *chosen) in enumerate(series):
        valid = [
            (x, y)
            for x, y in xy
            if x is not None and y is not None and math.isfinite(x) and math.isfinite(y)
        ]
        # Keep the report bounded while preserving endpoints.
        if valid:
            endpoint = valid[-1]
            valid = valid[:: max(1, math.ceil(len(valid) / 3000))]
            if valid[-1] != endpoint:
                valid.append(endpoint)
        points = "observed" in name.lower() or "residual" in name.lower()
        if points:
            color, dash = _COLOURS["ink"], ""
        elif chosen:
            color, dash = chosen[0]
        else:
            color, dash = _series_style(lines)
            lines += 1
        pattern = f' stroke-dasharray="{dash}"' if dash else ""
        if valid:
            coordinates = " ".join(
                f"{60+480*(x-x0)/dx:.2f},{250-190*(y-y0)/dy:.2f}" for x, y in valid
            )
            if points:
                content.extend(
                    f'<circle cx="{60+480*(x-x0)/dx:.2f}" cy="{250-190*(y-y0)/dy:.2f}" r="2" fill="{_COLOURS["surface"]}" stroke="{color}"/>'
                    for x, y in valid
                )
            else:
                content.append(
                    f'<polyline points="{coordinates}" fill="none" stroke="{color}" stroke-width="1.8"{pattern}/>'
                )
        swatch = (f'<circle cx="9" cy="5" r="3" fill="{_COLOURS["surface"]}" stroke="{color}"/>' if points
                  else f'<line x1="0" y1="5" x2="18" y2="5" stroke="{color}" stroke-width="2"{pattern}/>')
        legends.append(f'<span><svg class="key" viewBox="0 0 18 10" aria-hidden="true">{swatch}</svg>{escape(name)}</span>')
    return (
        f'<figure><figcaption>{escape(title)}</figcaption><svg viewBox="0 0 600 300" role="img" aria-label="{escape(title, quote=True)}">'
        f'<path d="M60 55 V250 H550" stroke="{_COLOURS["muted"]}" fill="none"/>'
        + "".join(content)
        + f'<text x="60" y="267" font-size="11">{x0:.3g}</text><text x="520" y="267" font-size="11">{x1:.3g}</text>'
        f'<text x="5" y="60" font-size="11">{y1:.3g}</text><text x="5" y="250" font-size="11">{y0:.3g}</text>'
        f'<text x="220" y="290" font-size="12">{escape(xlabel)}</text>'
        f'<text x="15" y="155" transform="rotate(-90 15 155)" text-anchor="middle" font-size="11">{escape(ylabel)}</text></svg><div class="legend">'
        + "".join(legends)
        + "</div></figure>"
    )


def write_analysis_report(
    path,
    *,
    title="QSPQuest analysis",
    fit=None,
    vpc=None,
    comparison=None,
    profile=None,
    bootstrap=None,
    runs=None,
):
    """Save a standalone report with curves, estimates, diagnostics and limits.

    ``comparison`` is a compare_regimens result, or a dict of them keyed by
    model to overlay the models' median trajectories. ``runs`` is a dict of
    labelled run results or saved tables (``qspquest.read_table``), for
    example one per model; each output they share is drawn in one plot.
    Colours follow the figure style shared with ``qspquest.plot``.
    """
    sections = []
    if runs is not None:
        sections.extend(_run_sections(runs))
    if fit is not None:
        from .diagnostics import fit_diagnostics
        from .friendly import scientific_status

        status = scientific_status(fit)
        sections.append(
            f'<p class="warning">{escape(status["title"])}: {escape(status["message"])}</p>'
        )
        diagnostics = (
            fit_diagnostics(fit)
            if fit.get("fitted_observations")
            else {
                "observations": [],
                "warnings": [
                    "No final prediction rows are available; retained estimates are exploratory."
                ],
                "population_diagnostics": fit.get("diagnostics"),
            }
        )

        sections.append(f'<h2>Fit: {escape(fit["status"])}</h2>')
        sections.append(
            "<table><tr><th>Parameter</th><th>Estimate</th><th>Unit</th></tr>"
            + "".join(
                f'<tr><td>{escape(k)}</td><td>{q["value"]:.6g}</td><td>{escape(q["unit"])}</td></tr>'
                for k, q in fit["parameters"].items()
            )
            + "</table>"
        )
        rows = diagnostics["observations"]
        unit = rows[0]["unit"] if rows else "unavailable"
        pred = lambda r: r.get("individual_prediction", r["prediction"])
        sections.append(
            _plot(
                "Observed versus predicted",
                [
                    (
                        "observed",
                        [
                            (pred(r), r["observed"])
                            for r in rows
                            if r.get("censoring") is None
                        ],
                    )
                ],
                f"Predicted ({unit})",
                f"Observed ({unit})",
            )
        )
        sections.append(
            _plot(
                "Standardized residuals",
                [("residual", [(r["time"], r["standardized_residual"]) for r in rows])],
                "Time",
                "Conditional standardized residual",
            )
        )
        trace = fit.get("trace", [])
        for axis, name in enumerate(fit["parameters"]):
            series = [
                (
                    name,
                    [
                        (
                            r.get("iteration", r.get("simulation")),
                            (
                                r["fixed_effects"][axis]
                                if "fixed_effects" in r
                                else r["parameters"][name]
                            ),
                        )
                        for r in trace
                    ],
                )
            ]
            sections.append(
                _plot(
                    f"Parameter trace: {name}",
                    series,
                    "Iteration / simulation",
                    fit["parameters"][name]["unit"],
                )
            )
        for warning in diagnostics["warnings"]:
            sections.append(f'<p class="warning">{escape(warning)}</p>')
        if diagnostics["population_diagnostics"]:
            sections.append(
                "<h3>Population mixing and shrinkage</h3><pre>"
                + escape(
                    json.dumps(
                        diagnostics["population_diagnostics"], indent=2, allow_nan=False
                    )
                )
                + "</pre>"
            )
    if vpc is not None:
        sections.append("<h2>Visual predictive check</h2>")
        sections.append(
            "<p>Bands span simulated cohort quantiles conditional on the supplied population parameters, variability and residual-error settings. "
            "They do not propagate parameter-estimation uncertainty.</p>"
        )
        sections.append('<p>Status: ' + escape(str(vpc.get('status', 'not_reported')))
                        + '; successful replicates: ' + escape(str(vpc.get('successful_replicates', 'not_reported')))
                        + '; failed replicates: ' + escape(str(vpc.get('failed_replicates', 'not_reported'))) + '.</p>')
        if vpc.get('failed_replicates', 0):
            sections.append('<p>Incomplete predictive experiment. Replicate bands are suppressed; successful-draw summaries are diagnostic only.</p>')
        if 'replicate_records' in vpc:
            sections.append('<details><summary>Replicate status and random-stream receipts</summary><pre>'
                            + escape(json.dumps(vpc['replicate_records'], indent=2, allow_nan=False)) + '</pre></details>')
        sections.append(
            f'<p>LLOQ supplied for {vpc["lloq_coverage"]} of {vpc["total_observations"]} observations. BLQ fractions describe supplied limits only.</p>'
        )
        for stratum in sorted({b["stratum"] for b in vpc["bins"]}):
            bins = [b for b in vpc["bins"] if b["stratum"] == stratum]
            series = []
            for j, q in enumerate(bins[0]["quantiles"]):
                for kind in ("observed", "simulated"):
                    series.append(
                        (
                            f'{kind} p={q["percentile"]:g}',
                            [
                                ((b["start"] + b["end"]) / 2, b["quantiles"][j][kind])
                                for b in bins
                            ],
                        )
                    )
                for end, label in ((0, "lower"), (1, "upper")):
                    series.append(
                        (
                            f'{label} replicate band p={q["percentile"]:g}',
                            [
                                (
                                    (b["start"] + b["end"]) / 2,
                                    (
                                        b["quantiles"][j]["confidence_interval"][end]
                                        if b["quantiles"][j]["confidence_interval"]
                                        is not None
                                        else None
                                    ),
                                )
                                for b in bins
                            ],
                        )
                    )
            sections.append(
                _plot(f"VPC {stratum}", series, vpc["time_unit"], vpc["unit"])
            )
            if vpc["lloq_coverage"]:
                sections.append(
                    _plot(
                        "BLQ fraction",
                        [
                            (
                                kind,
                                [
                                    (
                                        (b["start"] + b["end"]) / 2,
                                        b[f"{kind}_blq_fraction"],
                                    )
                                    for b in bins
                                ],
                            )
                            for kind in ("observed", "simulated")
                        ],
                        vpc["time_unit"],
                        "Fraction below LLOQ",
                    )
                )
    if comparison is not None:
        sections.extend(_comparison_sections(comparison))
    if profile is not None:
        sections.append("<h2>Likelihood profile</h2>")
        sections.append(
            _plot(
                profile["parameter"],
                [
                    (
                        "profile",
                        [(r["value"], r["delta_nll"]) for r in profile["profile"]],
                    )
                ],
                profile["unit"],
                "Change in negative log likelihood",
            )
        )
        sections.append(
            f'<p>Approximate interval: {escape(str(profile["interval"]))}. Open endpoints are unresolved.</p>'
        )
    if bootstrap is not None:
        sections.append(
            f'<h2>Subject bootstrap</h2><p>{bootstrap["successful_replicates"]} successful; {bootstrap["failed_replicates"]} failed or nonconverged replicates.</p>'
        )
        sections.append(
            "<pre>" + escape(json.dumps(bootstrap["intervals"], indent=2)) + "</pre>"
        )
    if not sections:
        fail("provide at least one scientific analysis result")
    document = (
        '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>'
        + escape(title)
        + "</title><style>"
        + _css()
        + "</style><h1>"
        + escape(title)
        + "</h1>"
        + "".join(sections)
        + "<p>Research analysis. Local covariance, bootstrap and likelihood-ratio intervals require validation for the intended model and sampling design.</p></html>"
    )
    Path(path).write_text(document, encoding="utf-8")
    return str(Path(path).resolve())


def _css():
    c = _COLOURS
    font = ", ".join(f'"{name}"' for name in _STYLE["fonts"]["brand"]) + ", sans-serif"
    return (f"body{{font:16px {font};max-width:1050px;margin:40px auto;padding:0 20px;"
            f"color:{c['ink']};background:{c['surface']}}}"
            "figure{display:inline-block;width:min(100%,480px);margin:12px 8px}"
            "figcaption{font-weight:600}table{border-collapse:collapse}"
            f"td,th{{text-align:left;padding:8px 20px;border-bottom:1px solid {c['grid']}}}"
            f"pre{{overflow:auto;background:{c['grid']};padding:15px}}"
            f".warning{{font-weight:600;border-left:3px solid {c['muted']};padding-left:10px}}"
            ".legend{display:flex;flex-wrap:wrap;gap:6px 16px;font-size:12px}"
            "svg{width:100%}svg.key{width:18px;height:10px;margin-right:4px}"
            f"svg text{{fill:{c['ink']}}}")


def _comparisons(comparison):
    from .tables import collection_kind

    if collection_kind(comparison) == "regimens":
        return list(comparison.items())
    if not isinstance(comparison, dict) or comparison.get("schema") != "qspquest.regimen-comparison/v0.1":
        fail("comparison must be a compare_regimens result or a dict of them keyed by model")
    return [(None, comparison)]


def _middle(record):
    return min(range(len(record["percentiles"])), key=lambda i: abs(record["percentiles"][i] - 0.5))


def _comparison_sections(comparison):
    items = _comparisons(comparison)
    units = {(r["unit"], r["time_unit"]) for _, c in items for r in c["regimens"].values()}
    if len(units) != 1:
        fail("compared models must report the same output unit and time unit")
    unit, time_unit = units.pop()
    sections = ["<h2>Regimen comparison</h2>"]
    series, rows = [], []
    palette = _COLOURS["categorical"]
    if len(items) > len(palette):
        fail(f"at most {len(palette)} models can be overlaid")
    regimen_order = list(dict.fromkeys(name for _, result in items for name in result["regimens"]))
    for m, (model, result) in enumerate(items):
        for name, record in result["regimens"].items():
            label = name if model is None else f"{model}: {name}"
            curve = list(zip(record["times"], record["curve_percentiles"][_middle(record)]))
            if model is None:
                series.append((label, curve))
            else:  # colour identifies the model, line pattern the regimen
                pattern = _DASHES[regimen_order.index(name) % len(_DASHES)]
                series.append((label, curve, (palette[m], pattern)))
            rows.append((label, [record["metric_percentiles"][key][_middle(record)]
                                 for key in ("auc", "cmax", "endpoint")]))
    sections.append(_plot("Population median trajectories", series, time_unit, unit))
    sections.append(
        "<p>Exposure summaries integrate the supplied sampled trajectory without extrapolation. Refine the grid to assess integration error.</p>"
    )
    sections.append(
        "<table><tr><th>Regimen</th><th>Median AUC</th><th>Median peak</th><th>Median endpoint</th></tr>"
        + "".join(f"<tr><td>{escape(label)}</td>" + "".join(f"<td>{v:.6g}</td>" for v in values) + "</tr>"
                  for label, values in rows)
        + "</table>"
    )
    return sections


def _run_sections(runs):
    from .tables import collection_kind

    if collection_kind(runs) != "run" or not all(isinstance(k, str) for k in runs):
        fail("runs must be a dict of labelled run results or saved tables")
    time_units = {r["time_unit"] for r in runs.values()}
    if len(time_units) != 1:
        fail("runs must share one time unit")
    time_unit = time_units.pop()
    outputs = {}
    for label, result in runs.items():
        for column in result["outputs"]:
            outputs.setdefault(column["name"], []).append((label, result, column))
    sections = ["<h2>Trajectories</h2>"]
    for name, entries in outputs.items():
        units = {column["unit"] for _, _, column in entries}
        if len(units) != 1:
            fail(f"output {name} has different units across runs: {sorted(map(str, units))}")
        unit = units.pop()
        series = [(label, list(zip(result["times"], column["values"]))) for label, result, column in entries]
        sections.append(_plot(name, series, time_unit, unit if unit is not None else "unit not declared"))
    return sections
