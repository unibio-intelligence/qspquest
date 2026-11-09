"""Long-format pandas tables for QSPQuest results, with explicit unit columns.

Every table keeps the units recorded in the result. Values are not converted.
``read_table`` reads saved CSV tables with their manifests and needs only the
standard library; the other functions require pandas (``qspquest[plot]``).
"""

import csv
from decimal import Decimal, InvalidOperation
import hashlib
import io
import json
import math
from pathlib import Path
from statistics import NormalDist

from ._analysis import fail

_FIT_ROWS = (
    "subject", "time", "time_unit", "side", "output", "unit", "observed",
    "prediction", "individual_prediction", "residual", "residual_kind",
    "standardized_residual", "censored", "source_row",
)


def _pandas():
    try:
        import pandas as pd
    except ImportError as error:
        raise ImportError("Install qspquest[plot] to use result tables") from error
    return pd


def kind(result):
    """Return the result kind used by tables and figures, or fail if unknown.

    Kinds are run, table, fit, population, diagnostics, native_fit, vpc,
    profile, bootstrap, regimens, exposure, morris, scan, steady_state and
    event_sensitivities.
    """
    if not isinstance(result, dict):
        fail("expected a QSPQuest result dict")
    schema = result.get("schema")
    by_schema = {
        "qspquest.result/v0.1": "run",
        "qspquest.table/v1": "table",
        "qspquest.analysis-fit/v0.1": "fit",
        "qspquest.population-analysis/v0.1": "population",
        "qspquest.fit-diagnostics/v0.1": "diagnostics",
        "qspquest.vpc/v0.1": "vpc",
        "qspquest.profile/v0.1": "profile",
        "qspquest.bootstrap/v0.1": "bootstrap",
        "qspquest.regimen-comparison/v0.1": "regimens",
        "qspquest.morris-result/v0.1": "morris",
        "qspquest.scan-result/v0.1": "scan",
        "qspquest.event-sensitivities/v0.1": "event_sensitivities",
    }
    # Native analysis records are v0.2; earlier v0.1 records keep the same rows.
    by_schema.update({
        name.replace("/v0.1", "/v0.2"): kind
        for name, kind in list(by_schema.items())
        if kind in ("fit", "population", "diagnostics", "vpc", "profile", "bootstrap",
                    "regimens", "event_sensitivities")
    })
    if schema in by_schema:
        return by_schema[schema]
    if isinstance(schema, str) and schema.startswith("qspquest.fit-result/"):
        return "native_fit"
    if schema is None and {"result", "mapping", "fitted_observations"} <= set(result):
        return "native_fit"
    if schema is None and result.get("method") == "sampled_piecewise_linear_no_extrapolation":
        return "exposure"
    if schema is None and ("initial_states" in result or "steady_state" in result):
        return "steady_state"
    fail(f"unsupported result for tables and figures: schema {schema!r}")


_TABLES = {
    "run": ("trajectory",),
    "table": ("trajectory",),
    "fit": ("observations", "parameters", "trace"),
    "population": ("observations", "parameters", "random_effects",
                   "individual_parameters", "trace"),
    "diagnostics": ("observations",),
    "native_fit": ("observations", "parameters"),
    "vpc": ("bins", "blq"),
    "profile": ("profile", "parameters"),
    "bootstrap": ("replicates", "parameters"),
    "regimens": ("curves", "subjects", "summary"),
    "exposure": ("metrics",),
    "morris": ("effects", "elementary_effects"),
    "scan": ("trajectory", "parameters"),
    "steady_state": ("states",),
    "event_sensitivities": ("derivatives",),
}


def tables(result):
    """Return the table names available for a result; the first is the default."""
    found = collection_kind(result)
    if found is not None:
        return _TABLES[found]
    return _TABLES[kind(result)]


_SCHEMAS = {"qspquest.result/v0.1": "run", "qspquest.table/v1": "table",
            "qspquest.regimen-comparison/v0.1": "regimens",
            "qspquest.regimen-comparison/v0.2": "regimens"}


def collection_kind(result):
    """Return the kind of a labelled collection, or None if ``result`` is not one.

    A collection is a dict whose keys are labels, or (series, panel) label
    pairs, and whose values are all run results, saved tables or all regimen
    comparisons.
    """
    if not isinstance(result, dict) or not result or "schema" in result:
        return None
    kinds = set()
    for key, value in result.items():
        pair = isinstance(key, tuple) and len(key) == 2 and all(isinstance(k, str) for k in key)
        if not (isinstance(key, str) or pair) or not isinstance(value, dict):
            return None
        kinds.add(_SCHEMAS.get(value.get("schema")))
    if kinds <= {"run", "table"} and kinds:
        return "run"
    if kinds == {"regimens"}:
        return "regimens"
    return None


def _is_collection(result):
    return collection_kind(result) == "run"


def _labels(key):
    return (key, None) if isinstance(key, str) else key


def to_frame(result, table=None):
    """Return one long-format DataFrame for a result.

    ``table`` selects one of ``tables(result)``; the default is the first.
    A dict that maps labels to run results or saved tables gives one
    trajectory table with a ``group`` column; (series, panel) keys add a
    ``panel`` column. A dict that maps model labels to regimen comparisons
    adds a ``model`` column. Unit columns are always included.
    """
    pd = _pandas()
    found = collection_kind(result)
    if found is not None:
        if table not in (None, *_TABLES[found]):
            fail(f"this collection has tables {', '.join(_TABLES[found])}; not {table!r}")
        frames = []
        for key, value in result.items():
            group, panel = _labels(key)
            frame = to_frame(value, table)
            if found == "regimens":
                frames.append(frame.assign(model=group))
            else:
                frames.append(frame.assign(group=group, panel=panel))
        frame = pd.concat(frames, ignore_index=True)
        column = "model" if found == "regimens" else "group"
        frame[column] = pd.Categorical(frame[column], list(dict.fromkeys(_labels(k)[0] for k in result)))
        if found == "run":
            if frame["panel"].isna().all():
                frame = frame.drop(columns="panel")
            elif frame["panel"].isna().any():
                fail("use (series, panel) keys for every entry or for none")
            else:
                frame["panel"] = pd.Categorical(frame["panel"],
                                                list(dict.fromkeys(_labels(k)[1] for k in result)))
        return frame
    name = kind(result)
    available = _TABLES[name]
    table = available[0] if table is None else table
    if table not in available:
        fail(f"{name} results have tables {', '.join(available)}; not {table!r}")
    rows = _BUILDERS[(name, table)](result)
    columns = _COLUMNS.get(table)
    frame = pd.DataFrame(rows, columns=columns if not rows else None)
    if columns is not None and rows:
        frame = frame[[c for c in columns if c in frame.columns]
                      + [c for c in frame.columns if c not in columns]]
    return frame


_COLUMNS = {
    "trajectory": ["row", "time", "side", "time_unit", "output", "unit", "value"],
    "observations": list(_FIT_ROWS),
    "parameters": ["parameter", "estimate", "unit", "lower", "upper", "interval_method",
                   "confidence"],
}


def _number(value):
    return math.nan if value is None else float(value)


def _trajectory(result, extra=None):
    rows = []
    for column in result["outputs"]:
        for i, (time, side, value) in enumerate(zip(result["times"], result["sides"], column["values"])):
            rows.append({**(extra or {}), "row": i, "time": float(time), "side": side,
                         "time_unit": result["time_unit"], "output": column["name"],
                         "unit": column["unit"], "value": _number(value)})
    return rows


def _flat_observations(rows, *, residual_kind):
    result = []
    for r in rows:
        individual = r.get("individual_prediction")
        result.append({
            "subject": str(r.get("subject_id", "1")),
            "time": float(r["time"]),
            "time_unit": r.get("time_unit"),
            "side": r.get("side"),
            "output": r["output"],
            "unit": r["unit"],
            "observed": float(r["observed"]),
            "prediction": float(r["prediction"]),
            "individual_prediction": _number(individual),
            "residual": _number(r.get("residual")),
            "residual_kind": residual_kind(r),
            "standardized_residual": _number(r.get("standardized_residual")),
            "censored": r.get("censoring") == "left",
            "source_row": r.get("source_row"),
        })
    return result


def _fit_observations(result):
    rows = result.get("fitted_observations") or []
    return _flat_observations(rows, residual_kind=lambda r: "observed_minus_prediction")


def _population_observations(result):
    rows = result.get("fitted_observations") or []
    return _flat_observations(rows, residual_kind=lambda r: "observed_minus_individual_prediction")


def _diagnostic_observations(result):
    def residual(r):
        return ("observed_minus_individual_prediction" if "individual_prediction" in r
                else "observed_minus_prediction")
    return _flat_observations(result["observations"], residual_kind=residual)


def _quantity(value, name):
    if not isinstance(value, dict) or "value" not in value or "unit" not in value:
        fail(f"{name} must be a quantity with value and unit")
    return float(value["value"]), value["unit"]


def _native_observations(result):
    ids = None
    if "mapping" in result:
        rows = result.get("fitted_observations")
        ids = result.get("subject_ids")
    else:
        rows = (result.get("population") or {}).get("fitted_observations")
    if not rows:
        fail("this native fit has no fitted observation rows; only population fits return them")
    out = []
    for r in rows:
        observed, unit = _quantity(r["value"], "observed value")
        prediction, prediction_unit = _quantity(r["pred"], "population prediction")
        individual, individual_unit = _quantity(r["ipred"], "individual prediction")
        time, time_unit = _quantity(r["time"], "observation time")
        if not unit == prediction_unit == individual_unit:
            fail("observed and predicted values have different units")
        subject = r.get("subject_id")
        if subject is None:
            subject = ids[r["subject_index"]] if ids else r["subject_index"] + 1
        out.append({
            "subject": str(subject), "time": time, "time_unit": time_unit, "side": r.get("side"),
            "output": r["output"], "unit": unit, "observed": observed, "prediction": prediction,
            "individual_prediction": individual, "residual": observed - individual,
            "residual_kind": "observed_minus_individual_prediction",
            "standardized_residual": math.nan, "censored": False, "source_row": r.get("source_row"),
        })
    return out


def _point_parameters(parameters, method=None):
    return [{"parameter": name, "estimate": float(q["value"]), "unit": q["unit"],
             "lower": math.nan, "upper": math.nan, "interval_method": method, "confidence": math.nan}
            for name, q in parameters.items()]


def _fit_parameters(result):
    return _point_parameters(result["parameters"])


def _native_parameters(result):
    if "mapping" in result:
        result = result["result"]
    population = result.get("population")
    if population is None:
        fit = result.get("fit") or {}
        return _point_parameters(fit.get("parameters", {}))
    rows = _point_parameters(population["fixed_effects"])
    intervals = population.get("fixed_effect_intervals")
    if intervals:
        by_name = {i["name"]: i for i in intervals["intervals"]}
        for row in rows:
            interval = by_name.get(row["parameter"])
            if interval is not None:
                if interval["unit"] != row["unit"]:
                    fail("fixed-effect interval unit differs from its estimate")
                row.update(lower=float(interval["lower"]), upper=float(interval["upper"]),
                           interval_method=intervals["method"],
                           confidence=float(intervals["confidence_level"]))
    return rows


def _fit_trace(result):
    rows = []
    for record in result.get("trace", []):
        for name, value in record["parameters"].items():
            rows.append({"step": record["simulation"], "step_kind": "simulation",
                         "parameter": name, "value": float(value),
                         "unit": result["parameters"][name]["unit"],
                         "objective": float(record["objective"])})
    return rows


def _population_trace(result):
    names = result["random_effects"]
    rows = []
    for record in result.get("trace", []):
        for name, value in zip(names, record["fixed_effects"]):
            rows.append({"step": record["iteration"], "step_kind": "iteration",
                         "parameter": name, "value": float(value),
                         "unit": result["parameters"][name]["unit"], "objective": math.nan})
    return rows


def _random_effects(result):
    names = result["random_effects"]
    shrinkage = (result.get("diagnostics") or {}).get("eta_variance_shrinkage") or [None] * len(names)
    rows = []
    for subject, eta in zip(result["subject_ids"], result["eta"]):
        for j, name in enumerate(names):
            rows.append({"subject": str(subject), "effect": name, "eta": float(eta[j]),
                         "scale": "log", "parameter_unit": result["parameters"][name]["unit"],
                         "variance": float(result["omega"][j][j]),
                         "shrinkage": _number(shrinkage[j])})
    return rows


def _individual_parameters(result):
    rows = []
    for subject, values in zip(result["subject_ids"], result["individual_parameters"]):
        for name, value in values.items():
            rows.append({"subject": str(subject), "parameter": name, "value": float(value),
                         "unit": result["parameters"][name]["unit"],
                         "estimate_kind": result.get("individual_estimate_kind")})
    return rows


def _vpc_bins(result):
    rows = []
    for record in result["bins"]:
        for q in record["quantiles"]:
            interval = q.get("confidence_interval") or (None, None)
            rows.append({
                "stratum": record["stratum"], "start": record["start"], "end": record["end"],
                "midpoint": (record["start"] + record["end"]) / 2,
                "observations": record["observations"], "percentile": q["percentile"],
                "observed": _number(q["observed"]), "simulated": _number(q["simulated"]),
                "lower": _number(interval[0]), "upper": _number(interval[1]),
                "quantifiable_replicates": q["quantifiable_replicates"],
                "output": result["output"], "unit": result["unit"], "time_unit": result["time_unit"],
                "prediction_corrected": result["prediction_corrected"],
                "confidence": result["confidence"],
            })
    return rows


def _vpc_blq(result):
    rows = []
    for record in result["bins"]:
        interval = record.get("blq_confidence_interval") or (None, None)
        rows.append({
            "stratum": record["stratum"], "start": record["start"], "end": record["end"],
            "midpoint": (record["start"] + record["end"]) / 2,
            "observations": record["observations"],
            "observed": record["observed_blq_fraction"],
            "simulated": _number(record["simulated_blq_fraction"]),
            "lower": _number(interval[0]), "upper": _number(interval[1]),
            "time_unit": result["time_unit"], "confidence": result["confidence"],
        })
    return rows


def _profile(result):
    lower, upper = result["interval"]
    return [{"parameter": result["parameter"], "value": float(r["value"]), "unit": result["unit"],
             "delta_nll": _number(r["delta_nll"]), "objective": _number(r["objective"]),
             "status": r["status"], "threshold": result["delta_nll_threshold"],
             "estimate": result["estimate"], "lower": _number(lower), "upper": _number(upper),
             "confidence": result["confidence"]}
            for r in result["profile"]]


def _profile_parameters(result):
    lower, upper = result["interval"]
    return [{"parameter": result["parameter"], "estimate": float(result["estimate"]),
             "unit": result["unit"], "lower": _number(lower), "upper": _number(upper),
             "interval_method": result["interval_method"], "confidence": result["confidence"]}]


def _bootstrap_replicates(result):
    rows = []
    for record in result["replicates"]:
        for name, q in (record.get("parameters") or {}).items():
            rows.append({"replicate": record["replicate"], "status": record.get("status"),
                         "parameter": name, "value": float(q["value"]), "unit": q["unit"]})
    return rows


def _bootstrap_parameters(result):
    successful = [r for r in result["replicates"] if r.get("status") == "converged"]
    intervals = result.get("intervals") or {}
    names = list(intervals) or list((successful[0]["parameters"] if successful else {}))
    rows = []
    for name in names:
        values = sorted(float(r["parameters"][name]["value"]) for r in successful)
        unit = intervals[name]["unit"] if name in intervals else successful[0]["parameters"][name]["unit"]
        median = _median(values)
        bounds = intervals.get(name, {}).get("bounds", (None, None))
        rows.append({"parameter": name, "estimate": median, "unit": unit,
                     "lower": _number(bounds[0]), "upper": _number(bounds[1]),
                     "interval_method": result["method"] if name in intervals else None,
                     "confidence": result["confidence"], "estimate_kind": "bootstrap_median"})
    return rows


def _median(values):
    if not values:
        return math.nan
    n = len(values)
    return values[n // 2] if n % 2 else (values[n // 2 - 1] + values[n // 2]) / 2


def _regimen_curves(result):
    rows = []
    for label, record in result["regimens"].items():
        for percentile, values in zip(record["percentiles"], record["curve_percentiles"]):
            for i, (time, side, value) in enumerate(zip(record["times"], record["sides"], values)):
                rows.append({"regimen": label, "row": i, "time": float(time), "side": side,
                             "time_unit": record["time_unit"], "unit": record["unit"],
                             "percentile": percentile, "value": float(value)})
    return rows


def _metric_unit(metric, record):
    if metric == "auc":
        return f"{record['unit']} * {record['time_unit']}"
    if metric in ("tmax", "time_above_threshold"):
        return record["time_unit"]
    return record["unit"]


def _regimen_subjects(result):
    rows = []
    for label, record in result["regimens"].items():
        for subject, metrics in enumerate(record["subject_metrics"]):
            for metric in ("auc", "cmax", "tmax", "cmin", "endpoint"):
                rows.append({"regimen": label, "subject": subject + 1, "metric": metric,
                             "value": float(metrics[metric]), "unit": _metric_unit(metric, record)})
            above = metrics.get("time_above_threshold")
            if above is not None:
                rows.append({"regimen": label, "subject": subject + 1,
                             "metric": "time_above_threshold", "value": float(above["duration"]),
                             "unit": record["time_unit"]})
    return rows


def _regimen_summary(result):
    rows = []
    for label, record in result["regimens"].items():
        for metric, values in record["metric_percentiles"].items():
            for percentile, value in zip(record["percentiles"], values):
                rows.append({"regimen": label, "metric": metric, "percentile": percentile,
                             "value": float(value), "unit": _metric_unit(metric, record),
                             "statistic": "subject_percentile"})
        for metric, values in record.get("paired_differences_vs_reference", {}).items():
            for percentile, value in zip(record["percentiles"], values):
                rows.append({"regimen": label, "metric": metric, "percentile": percentile,
                             "value": float(value), "unit": _metric_unit(metric, record),
                             "statistic": "paired_difference_vs_reference"})
    return rows


def _exposure(result):
    rows = []
    for metric in ("auc", "cmax", "tmax", "cmin", "endpoint"):
        rows.append({"output": result["output"], "metric": metric, "value": float(result[metric]),
                     "unit": _metric_unit(metric, result)})
    above = result.get("time_above_threshold")
    if above is not None:
        rows.append({"output": result["output"], "metric": "time_above_threshold",
                     "value": float(above["duration"]), "unit": result["time_unit"]})
    return rows


def _morris_effects(result):
    return [{"parameter": e["parameter"], "mu": float(e["mu"]), "mu_star": float(e["mu_star"]),
             "sigma": float(e["sigma"]), "unit": e["unit"], "trajectories": len(e["effects"])}
            for e in result["result"]["effects"]]


def _morris_elementary(result):
    return [{"parameter": e["parameter"], "trajectory": i, "effect": float(value), "unit": e["unit"]}
            for e in result["result"]["effects"] for i, value in enumerate(e["effects"])]


def _scan_trajectory(result):
    rows = []
    for point in result["results"]:
        rows.extend(_trajectory(point["result"], {"point": point["id"]}))
    return rows


def _scan_parameters(result):
    return [{"point": point["id"], "parameter": name, "value": float(q["value"]), "unit": q["unit"]}
            for point in result["results"] for name, q in point["parameters"].items()]


def _steady_states(result):
    cycles = None
    if "steady_state" in result:
        cycles = result.get("cycles_completed")
        result = result["steady_state"]
    rows = []
    for name, q in result["initial_states"].items():
        residual = result["residuals"][name]
        rows.append({"state": name, "amount": float(q["value"]), "unit": q["unit"],
                     "residual": float(residual["value"]), "residual_unit": residual["unit"],
                     "maximum_scaled_residual": float(result["maximum_scaled_residual"]),
                     "cycles_completed": cycles})
    return rows


def _event_sensitivities(result):
    rows = []
    for parameter, outputs in result["derivatives"].items():
        for output, column in outputs.items():
            for i, (time, side, value) in enumerate(zip(result["times"], result["sides"], column["values"])):
                rows.append({"parameter": parameter, "output": output, "row": i, "time": float(time),
                             "side": side, "time_unit": result["time_unit"], "value": float(value),
                             "unit": column["unit"]})
    return rows


_BUILDERS = {
    ("run", "trajectory"): _trajectory,
    ("table", "trajectory"): _trajectory,
    ("fit", "observations"): _fit_observations,
    ("fit", "parameters"): _fit_parameters,
    ("fit", "trace"): _fit_trace,
    ("population", "observations"): _population_observations,
    ("population", "parameters"): _fit_parameters,
    ("population", "random_effects"): _random_effects,
    ("population", "individual_parameters"): _individual_parameters,
    ("population", "trace"): _population_trace,
    ("diagnostics", "observations"): _diagnostic_observations,
    ("native_fit", "observations"): _native_observations,
    ("native_fit", "parameters"): _native_parameters,
    ("vpc", "bins"): _vpc_bins,
    ("vpc", "blq"): _vpc_blq,
    ("profile", "profile"): _profile,
    ("profile", "parameters"): _profile_parameters,
    ("bootstrap", "replicates"): _bootstrap_replicates,
    ("bootstrap", "parameters"): _bootstrap_parameters,
    ("regimens", "curves"): _regimen_curves,
    ("regimens", "subjects"): _regimen_subjects,
    ("regimens", "summary"): _regimen_summary,
    ("exposure", "metrics"): _exposure,
    ("morris", "effects"): _morris_effects,
    ("morris", "elementary_effects"): _morris_elementary,
    ("scan", "trajectory"): _scan_trajectory,
    ("scan", "parameters"): _scan_parameters,
    ("steady_state", "states"): _steady_states,
    ("event_sensitivities", "derivatives"): _event_sensitivities,
}


# Saved CSV tables -----------------------------------------------------------

TABLE_MANIFEST = "qspquest.table-manifest/v1"
_MAX_CSV = 256 * 1024 * 1024
_MAX_MANIFEST = 16 * 1024 * 1024


def _unique_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            fail("duplicate manifest key: " + key)
        result[key] = value
    return result


def _bounded(path, limit, what):
    if path.is_symlink() or not path.is_file():
        fail(f"{what} must be an ordinary file: {path}")
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        fail(f"{what} exceeds its size limit")
    return raw


def read_table(path):
    """Read a saved CSV table and its ``.manifest.json``.

    Accepts tables written by the MCP ``save_csv`` tool, the CLI or the Python
    first run (``qspquest.table-manifest/v1``). The CSV byte count and SHA-256,
    header, row and event counts, finite values and the pre-then-post order of
    equal times are checked against the manifest. Units, column meanings and
    model identity are taken from the manifest; values are not converted.
    Outputs are named by their native model name where the manifest records
    one, otherwise by the CSV column. The returned table plots and converts
    like a run result, and the full manifest is kept under ``manifest``. Receipts in the manifest are kept but
    not re-validated here.
    """
    path = Path(path)
    raw_manifest = _bounded(Path(str(path) + ".manifest.json"), _MAX_MANIFEST, "table manifest")

    def nonfinite(value):
        fail("table manifest contains a nonfinite number: " + value)

    try:
        manifest = json.loads(raw_manifest, object_pairs_hook=_unique_keys, parse_constant=nonfinite)
    except json.JSONDecodeError as error:
        fail(f"table manifest is not valid JSON: {error}")
    if not isinstance(manifest, dict) or manifest.get("schema") != TABLE_MANIFEST:
        fail(f"table manifest schema must be {TABLE_MANIFEST}")
    table = manifest.get("table")
    if not isinstance(table, dict) or table.get("filename") != path.name:
        fail("the manifest names a different CSV file")
    columns = table.get("columns")
    if (not isinstance(columns, list) or len(columns) < 3
            or any(not isinstance(c, dict) or not isinstance(c.get("name"), str) for c in columns)):
        fail("the manifest needs time, side and at least one value column")
    if columns[0].get("kind") != "time" or columns[1].get("kind") != "event_side":
        fail("the first two table columns must be time and event side")
    time_unit = columns[0].get("unit")
    if not isinstance(time_unit, str) or not time_unit:
        fail("the table manifest records no time unit")
    names = [c["name"] for c in columns]
    if len(set(names)) != len(names):
        fail("table column names must be unique")
    raw = _bounded(path, _MAX_CSV, "table CSV")
    if len(raw) != table.get("bytes") or hashlib.sha256(raw).hexdigest() != table.get("sha256"):
        fail("the CSV byte count or SHA-256 differs from its manifest")
    reader = csv.reader(io.StringIO(raw.decode("utf-8"), newline=""), strict=True)
    times, sides, values = [], [], [[] for _ in columns[2:]]
    counts = {"pre": 0, "post": 0, "event_pairs": 0}
    previous = paired = None
    try:
        if next(reader, None) != names:
            fail("the CSV header differs from the manifest columns")
        for row in reader:
            if len(row) != len(names):
                fail("a CSV row has the wrong number of cells")
            time, side = Decimal(row[0]), row[1]
            if not time.is_finite() or side not in ("pre", "post"):
                fail("invalid CSV time or event side")
            if previous is not None and time < previous[0]:
                fail("CSV times must be nondecreasing")
            if previous is not None and time == previous[0]:
                if paired or (previous[1], side) != ("pre", "post"):
                    fail("rows at one time must be exactly pre then post")
                paired = True
                counts["event_pairs"] += 1
            else:
                paired = False
            previous = (time, side)
            counts[side] += 1
            times.append(float(time))
            sides.append(side)
            for column, cell in zip(values, row[2:]):
                number = float(cell)
                if not math.isfinite(number):
                    fail("CSV values must be finite")
                column.append(number)
    except (csv.Error, InvalidOperation, ValueError) as error:
        if getattr(error, "code", None):
            raise
        fail(f"malformed CSV: {error}")
    events = manifest.get("events") or {}
    if len(times) != table.get("rows") or any(events.get(k) != v for k, v in counts.items()):
        fail("the CSV row or event counts differ from its manifest")
    model = manifest.get("model") or {}
    native = [c.get("native_name") or c["name"] for c in columns[2:]]
    if len(set(native)) != len(native):
        native = names[2:]
    return {
        "schema": "qspquest.table/v1",
        "path": str(path),
        "model_id": model.get("id"),
        "time_unit": time_unit,
        "times": times,
        "sides": sides,
        "outputs": [{"name": name, "column": c["name"], "unit": c.get("unit"),
                     "kind": c.get("kind"), "meaning": c.get("meaning"), "values": column}
                    for name, c, column in zip(native, columns[2:], values)],
        "manifest": manifest,
    }


# Target attainment ----------------------------------------------------------

def _comparisons(result):
    found = collection_kind(result)
    if found == "regimens":
        return [(_labels(key)[0], value) for key, value in result.items()]
    if kind(result) != "regimens":
        fail("target attainment needs a compare_regimens result or a dict of them")
    return [(None, result)]


def wilson_interval(successes, total, confidence=0.95):
    """Wilson score interval for a binomial proportion."""
    if not 0 < confidence < 1 or total <= 0 or not 0 <= successes <= total:
        fail("Wilson interval needs 0 <= successes <= total, total > 0 and confidence in (0, 1)")
    z = NormalDist().inv_cdf(1 - (1 - confidence) / 2)
    p = successes / total
    centre = (p + z * z / (2 * total)) / (1 + z * z / total)
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / (1 + z * z / total)
    return max(0.0, centre - half), min(1.0, centre + half)


def target_attainment(result, metric, target, *, direction="below", confidence=0.95):
    """Fraction of virtual subjects whose metric meets a target, per regimen.

    ``result`` is a compare_regimens result or a dict of them keyed by model.
    ``metric`` is a per-subject metric (auc, cmax, cmin, tmax, endpoint or
    time_above_threshold). ``direction='below'`` counts subjects with
    metric <= target and ``'above'`` those with metric >= target. Intervals
    are Wilson score intervals. Metrics cover the sampled window of the
    comparison, so a window maximum below a target is ``cmax`` with
    ``direction='below'``.
    """
    pd = _pandas()
    if direction not in ("below", "above"):
        fail("direction must be 'below' or 'above'")
    if isinstance(target, bool) or not isinstance(target, (int, float)) or not math.isfinite(target):
        fail("target must be a finite number in the metric's unit")
    rows = []
    for model, comparison in _comparisons(result):
        frame = to_frame(comparison, "subjects")
        if metric not in set(frame["metric"]):
            fail(f"metric must be one of {', '.join(dict.fromkeys(frame['metric']))}")
        frame = frame[frame["metric"] == metric]
        for regimen, part in frame.groupby("regimen", sort=False):
            values = part["value"]
            met = int((values <= target).sum() if direction == "below" else (values >= target).sum())
            lower, upper = wilson_interval(met, len(values), confidence)
            rows.append({"model": model, "regimen": regimen, "metric": metric,
                         "unit": part["unit"].iloc[0], "target": float(target),
                         "direction": direction, "met": met, "subjects": len(values),
                         "attainment": met / len(values), "lower": lower, "upper": upper,
                         "confidence": confidence, "interval_method": "wilson"})
    table = pd.DataFrame(rows)
    if table["unit"].nunique() != 1:
        fail("the comparisons report the metric in different units")
    if table["model"].isna().all():
        table = table.drop(columns="model")
    return table
