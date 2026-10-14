"""Portable SVG/HTML views of scientific analysis results, without network I/O."""

from html import escape
import json
import math
from pathlib import Path
from ._analysis import fail


def _plot(title, series, xlabel, ylabel):
    points = [
        (float(x), float(y))
        for _, xy in series
        for x, y in xy
        if x is not None and y is not None and math.isfinite(x) and math.isfinite(y)
    ]
    if not points:
        return ""
    x0, x1 = min(x for x, _ in points), max(x for x, _ in points)
    y0, y1 = min(y for _, y in points), max(y for _, y in points)
    dx, dy = max(x1 - x0, 1e-12), max(y1 - y0, 1e-12)
    colors = ["#2563eb", "#d97706", "#059669", "#9333ea", "#dc2626"]
    content, legends = [], []
    for i, (name, xy) in enumerate(series):
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
        color = colors[i % len(colors)]
        if valid:
            coordinates = " ".join(
                f"{60+480*(x-x0)/dx:.2f},{250-190*(y-y0)/dy:.2f}" for x, y in valid
            )
            if "observed" in name.lower() or "residual" in name.lower():
                content.extend(
                    f'<circle cx="{60+480*(x-x0)/dx:.2f}" cy="{250-190*(y-y0)/dy:.2f}" r="2" fill="{color}"/>'
                    for x, y in valid
                )
            else:
                content.append(
                    f'<polyline points="{coordinates}" fill="none" stroke="{color}" stroke-width="1.8"/>'
                )
        legends.append(f'<span style="color:{color}">{escape(name)}</span>')
    return (
        f'<figure><figcaption>{escape(title)}</figcaption><svg viewBox="0 0 600 300" role="img" aria-label="{escape(title, quote=True)}">'
        '<path d="M60 55 V250 H550" stroke="#64748b" fill="none"/>'
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
):
    """Save a standalone report with curves, estimates, diagnostics and limits."""
    sections = []
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
        sections.append('<p>Status: ' + escape(str(vpc.get('status', 'not_reported')))
                        + '; successful replicates: ' + escape(str(vpc.get('successful_replicates', 'not_reported')))
                        + '; failed replicates: ' + escape(str(vpc.get('failed_replicates', 'not_reported'))) + '.</p>')
        if vpc.get('failed_replicates', 0):
            sections.append('<p>Incomplete predictive experiment. Confidence intervals are suppressed; successful-draw summaries are diagnostic only.</p>')
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
                            f'{label} CI p={q["percentile"]:g}',
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
        sections.append("<h2>Regimen comparison</h2>")
        series = []
        for name, r in comparison["regimens"].items():
            j = min(
                range(len(r["percentiles"])),
                key=lambda i: abs(r["percentiles"][i] - 0.5),
            )
            series.append((name, list(zip(r["times"], r["curve_percentiles"][j]))))
        first = next(iter(comparison["regimens"].values()))
        sections.append(
            _plot(
                "Population median trajectories",
                series,
                first["time_unit"],
                first["unit"],
            )
        )
        sections.append(
            "<p>Exposure summaries integrate the supplied sampled trajectory without extrapolation. Refine the grid to assess integration error.</p>"
        )
        sections.append(
            "<table><tr><th>Regimen</th><th>Median AUC</th><th>Median peak</th><th>Median endpoint</th></tr>"
            + "".join(
                f"<tr><td>{escape(name)}</td>"
                + "".join(
                    f'<td>{r["metric_percentiles"][key][min(range(len(r["percentiles"])), key=lambda i:abs(r["percentiles"][i]-.5))]:.6g}</td>'
                    for key in ("auc", "cmax", "endpoint")
                )
                + "</tr>"
                for name, r in comparison["regimens"].items()
            )
            + "</table>"
        )
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
        + "</title><style>body{font:16px system-ui;max-width:1050px;margin:40px auto;padding:0 20px;color:#172033}figure{display:inline-block;width:min(100%,480px);margin:12px 8px}figcaption{font-weight:600}table{border-collapse:collapse}td,th{text-align:left;padding:8px 20px;border-bottom:1px solid #ddd}pre{overflow:auto;background:#f4f6f9;padding:15px}.warning{color:#92400e}.legend{display:flex;flex-wrap:wrap;gap:6px 16px;font-size:12px}svg{width:100%}</style><h1>"
        + escape(title)
        + "</h1>"
        + "".join(sections)
        + "<p>Research analysis. Local covariance, bootstrap and likelihood-ratio intervals require validation for the intended model and sampling design.</p></html>"
    )
    Path(path).write_text(document, encoding="utf-8")
    return str(Path(path).resolve())
