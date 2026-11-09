"""Pharmacometric figures built from QSPQuest results.

Each function takes a native QSPQuest result, reads axis labels and units from
that result, and returns a plotnine object that can be extended with ``+``.
Colours, line weights and sizes come only from the public API of
``qspquest.plot.style``.
"""

import math
import warnings

try:
    import pandas as pd
    from plotnine import (
        aes, coord_equal, element_blank, element_text, facet_wrap, geom_abline, guide_legend,
        guides, geom_histogram, geom_hline, geom_line, geom_path, geom_point, geom_rect,
        geom_ribbon, geom_segment, geom_text, geom_tile, geom_vline, ggplot, labs,
        scale_color_manual, scale_fill_manual, scale_linetype_manual, scale_shape_manual,
        scale_x_continuous, scale_x_log10, scale_y_continuous, scale_y_log10, theme,
    )
    from plotnine.composition import Beside, Stack
except ImportError as error:  # pragma: no cover - exercised without the extra
    raise ImportError("Install qspquest[plot] to make figures") from error

from .._analysis import fail
from ..tables import collection_kind, kind, target_attainment as _attainment_table, to_frame
from . import style

__all__ = [
    "profile", "observed_vs_predicted", "residuals", "goodness_of_fit", "vpc", "vpc_blq",
    "parameters", "bootstrap", "likelihood_profile", "regimens", "exposure", "exposure_ratio",
    "random_effects", "trace", "morris", "scan", "steady_state", "target_attainment",
]

_c = style.color


def _cat(index):
    return style.palette()[index]


def _many(count):
    return count > len(style.palette())


# Labels and validation ------------------------------------------------------

def _unit(unit):
    return unit.replace(" * ", "·").replace("*", "·")


def _label(name, unit):
    if unit is None:
        return f"{name} (unit not declared)"
    if unit in ("", "1", "dimensionless"):
        return name
    return f"{name} ({_unit(unit)})"


def _single(frame, column, what):
    values = frame[column].dropna().unique()
    if len(values) == 0:
        fail(f"the result records no {what}")
    if len(values) > 1:
        fail(f"the figure needs one {what}; found {', '.join(map(str, values))}")
    return values[0]


def _kind(result, allowed, function):
    found = {"run": "collection", "regimens": "model_comparisons"}.get(collection_kind(result))
    if found is None:
        found = kind(result)
    if found == "profile" and function == "profile":
        fail("this is a likelihood profile; use qspquest.plot.likelihood_profile(result)")
    if found not in allowed:
        fail(f"qspquest.plot.{function} accepts {', '.join(allowed)} results, not {found}")
    return found


def _positive(frame, columns, log, what="values"):
    if not log:
        return frame
    keep = (frame[columns] > 0).all(axis=1)
    dropped = int((~keep).sum())
    if dropped:
        warnings.warn(f"{dropped} non-positive {what} omitted from the logarithmic axis",
                      stacklevel=3)
    if not keep.any():
        fail(f"no positive {what} remain for the logarithmic axis")
    return frame[keep]


def _time_unit(frame, time_unit):
    recorded = frame["time_unit"].dropna().unique() if "time_unit" in frame else []
    if len(recorded) > 1:
        fail("rows use more than one time unit")
    if len(recorded) == 1:
        if time_unit is not None and time_unit != recorded[0]:
            fail(f"time_unit {time_unit!r} differs from the recorded unit {recorded[0]!r}")
        return recorded[0]
    if time_unit is None:
        fail("these rows record no time unit; pass time_unit= with the run's time unit")
    return time_unit


def _ordered(frame, column, levels):
    frame = frame.copy()
    frame[column] = pd.Categorical(frame[column], list(levels))
    return frame


def _percentile_name(p):
    if abs(p - 0.5) < 1e-12:
        return "median"
    value = round(100 * p, 1)
    text = f"{value:g}"
    suffix = "th"
    if not 10 <= value % 100 < 20 and float(text).is_integer():
        suffix = {1: "st", 2: "nd", 3: "rd"}.get(int(value) % 10, "th")
    return text + suffix + " percentile"


# Concentration-time profiles ------------------------------------------------

def profile(result, output=None, *, log=False, label=None, observed=None, subjects=None,
            time_unit=None):
    """Concentration-time (or any output-time) profile.

    Accepts a run result, a saved table from ``qspquest.read_table``, a dict of
    labelled runs or tables (one line each), a scan result (one line per
    point), or a fit, population fit, diagnostics or native population fit
    (observations as points, predictions as lines, one panel per subject).
    Dict keys may be (series, panel) pairs, for example (model, regimen):
    lines are then coloured by the first element and drawn in one panel per
    second element. ``observed`` adds points from a DataFrame with columns
    ``time``, ``value``, ``unit`` and ``time_unit`` (and optionally ``group``).
    Up to three series are coloured; more series are drawn in separate panels.
    """
    found = _kind(result, ("run", "table", "collection", "scan", "fit", "population",
                           "diagnostics", "native_fit", "regimens", "model_comparisons"),
                  "profile")
    if found in ("regimens", "model_comparisons"):
        return regimens(result, log=log, label=label)
    if found in ("fit", "population", "diagnostics", "native_fit"):
        if observed is not None:
            fail("fit results already contain their observations")
        return _fitted_profile(result, output, log, label, subjects, time_unit)
    if subjects is not None:
        fail("subjects= applies only to fit results")
    frame = to_frame(result)
    group = {"collection": "group", "scan": "point"}.get(found)
    if output is None:
        outputs = list(dict.fromkeys(frame["output"]))
        output = outputs[0] if len(outputs) == 1 else None
        if output is None:
            fail(f"the result has outputs {', '.join(outputs)}; choose one with output=")
    if output not in set(frame["output"]):
        fail(f"unknown output {output!r}")
    frame = frame[frame["output"] == output]
    unit = _output_unit(frame)
    time = _single(frame, "time_unit", "time unit")
    frame = _positive(frame, ["value"], log)
    name = label or output
    fig = ggplot(frame, aes("time", "value"))
    many = False
    if group is None:
        fig += geom_path(color=_cat(0), size=style.line("model"))
    elif "panel" in frame:
        levels = list(frame["group"].cat.categories)
        if _many(len(levels)):
            fail(f"at most {len(style.palette())} series can be coloured; "
                 "use them as panels instead")
        fig += geom_path(aes(color="group", group="group"), size=style.line("model"))
        fig += style.scale_color_categorical()
        fig += facet_wrap("~panel")
        many = True
    else:
        levels = list(dict.fromkeys(frame[group]))
        frame = _ordered(frame, group, levels)
        fig = ggplot(frame, aes("time", "value"))
        many = _many(len(levels))
        if many:
            fig += geom_path(aes(group=group), color=_cat(0), size=style.line("model"))
            fig += facet_wrap(f"~{group}")
        else:
            fig += geom_path(aes(color=group), size=style.line("model"))
            fig += style.scale_color_categorical()
    if observed is not None:
        points = _observed(observed, unit, time, log)
        if group is not None and "group" in points and not many:
            fig += geom_point(points, aes("time", "value", color="group"), fill=_c("surface"),
                              shape="o", size=style.point_size(), stroke=0.6)
        else:
            fig += geom_point(points, aes("time", "value"), color=_c("ink"), fill=_c("surface"),
                              shape="o", size=style.point_size(), stroke=0.6)
    if log:
        fig += scale_y_log10()
    fig += labs(x=_label("Time", time),
                y=_label(name, unit) + (", log scale" if log else ""))
    panels = frame["panel"].nunique() if "panel" in frame else 1
    if panels > 1:
        return style.size(fig + style.theme_style(), 180, 30 + 45 * math.ceil(panels / 3))
    return style.size(fig + style.theme_style(), 85, 65)


def _output_unit(frame):
    if frame["unit"].isna().all():
        return None
    if frame["unit"].isna().any():
        fail("some rows of this output declare no unit")
    return _single(frame, "unit", "output unit")


def _observed(observed, unit, time, log):
    if not isinstance(observed, pd.DataFrame):
        fail("observed must be a DataFrame with time, value, unit and time_unit columns")
    missing = {"time", "value", "unit", "time_unit"} - set(observed.columns)
    if missing:
        fail("observed is missing columns: " + ", ".join(sorted(missing)))
    if set(observed["unit"]) != {unit}:
        fail(f"observed values must use the output unit {unit!r}")
    if set(observed["time_unit"]) != {time}:
        fail(f"observed times must use the time unit {time!r}")
    return _positive(observed, ["value"], log, "observed values")


def _fit_frame(result, output, time_unit=None, *, need_time=True):
    frame = to_frame(result, "observations")
    if frame.empty:
        fail("the fit has no fitted observation rows")
    if output is None:
        outputs = list(dict.fromkeys(frame["output"]))
        if len(outputs) > 1:
            fail(f"the fit has outputs {', '.join(outputs)}; choose one with output=")
        output = outputs[0]
    if output not in set(frame["output"]):
        fail(f"unknown output {output!r}")
    frame = frame[frame["output"] == output].copy()
    unit = _single(frame, "unit", "output unit")
    time = None
    if need_time:
        time = _time_unit(frame, time_unit)
        frame["time_unit"] = time
    return frame, output, unit, time


def _fitted_profile(result, output, log, label, subjects, time_unit):
    frame, output, unit, time = _fit_frame(result, output, time_unit)
    if subjects is not None:
        wanted = [str(s) for s in subjects]
        unknown = set(wanted) - set(frame["subject"])
        if unknown:
            fail("unknown subjects: " + ", ".join(sorted(unknown)))
        frame = frame[frame["subject"].isin(wanted)].copy()
    frame["subject"] = "Subject " + frame["subject"]
    levels = list(dict.fromkeys(frame["subject"]))
    has_individual = frame["individual_prediction"].notna().any()
    lines = [("individual_prediction" if has_individual else "prediction",
              "Individual prediction" if has_individual else "Model prediction")]
    if has_individual:
        lines.append(("prediction", "Population prediction"))
    long = pd.concat([frame.assign(series=name, value=frame[column]) for column, name in lines],
                     ignore_index=True)
    long = _positive(long, ["value"], log, "predictions")
    points = frame.assign(value=frame["observed"],
                          series=frame["censored"].map({True: "Below LLOQ", False: "Observed"}))
    points = _positive(points, ["value"], log, "observations")
    names = [name for _, name in lines]
    long = _ordered(long, "subject", levels)
    points = _ordered(points, "subject", levels)
    long = _ordered(long, "series", names)
    fig = (ggplot(long, aes("time", "value"))
           + geom_path(aes(color="series", linetype="series", group="series"), size=style.line("model"))
           + geom_point(points, aes("time", "value", shape="series"), color=_c("ink"), fill=_c("surface"),
                        size=style.point_size(), stroke=0.6)
           + scale_color_manual(values=dict(zip(names, style.palette(len(names)))))
           + scale_linetype_manual(values=dict(zip(names, ["solid", "dashed"])))
           + scale_shape_manual(values={"Observed": "o", "Below LLOQ": "v"}))
    if len(levels) > 1:
        fig += facet_wrap("~subject")
    if log:
        fig += scale_y_log10()
    fig += labs(x=_label("Time", time),
                y=_label(label or output, unit) + (", log scale" if log else ""))
    columns = min(len(levels), 4)
    rows = math.ceil(len(levels) / columns)
    return style.size(fig + style.theme_style(), 180 if len(levels) > 1 else 85, 30 + 40 * rows if len(levels) > 1 else 70)


# Goodness of fit ------------------------------------------------------------

_PREDICTION_LABELS = {
    "prediction": "Population prediction",
    "individual_prediction": "Individual prediction",
    "model": "Model prediction",
}


def _prediction_column(frame, prediction):
    has_individual = frame["individual_prediction"].notna().any()
    if not has_individual:
        if prediction not in (None, "model"):
            fail("this fit has one prediction per row; use prediction='model'")
        return "prediction", _PREDICTION_LABELS["model"]
    choice = prediction or "individual"
    if choice not in ("population", "individual"):
        fail("prediction must be 'population' or 'individual' for population fits")
    column = "prediction" if choice == "population" else "individual_prediction"
    return column, _PREDICTION_LABELS[column]


def observed_vs_predicted(result, *, prediction=None, output=None, log=False, label=None):
    """Observed versus predicted values with the line of identity.

    ``prediction`` is 'individual' (default) or 'population' for population
    fits; individual fits have one model prediction. Rows below the LLOQ are
    omitted and counted in a warning.
    """
    _kind(result, ("fit", "population", "diagnostics", "native_fit"), "observed_vs_predicted")
    frame, output, unit, _ = _fit_frame(result, output, need_time=False)
    column, name = _prediction_column(frame, prediction)
    censored = int(frame["censored"].sum())
    if censored:
        warnings.warn(f"{censored} rows below the LLOQ omitted", stacklevel=2)
    frame = frame[~frame["censored"]].assign(x=lambda f: f[column], y=lambda f: f["observed"])
    frame = _positive(frame, ["x", "y"], log)
    low, high = min(frame["x"].min(), frame["y"].min()), max(frame["x"].max(), frame["y"].max())
    if not log:
        low = min(low, 0.0)
    fig = (ggplot(frame, aes("x", "y"))
           + geom_abline(intercept=0, slope=1, color=_c("muted"), size=style.line("reference"), linetype="dashed")
           + geom_point(color=_c("ink"), fill=_c("surface"), shape="o", size=style.point_size(), stroke=0.6))
    variable = label or output
    suffix = ", log scale" if log else ""
    if log:
        fig += scale_x_log10(limits=(low, high))
        fig += scale_y_log10(limits=(low, high))
    else:
        fig += scale_x_continuous(limits=(low, high))
        fig += scale_y_continuous(limits=(low, high))
    fig += coord_equal()
    fig += labs(x=_label(f"{name}, {variable}", unit) + suffix,
                y=_label(f"Observed {variable}", unit) + suffix)
    return style.size(fig + style.theme_style(), 80, 80)


_RESIDUAL_LABELS = {
    "observed_minus_prediction": "Residual, observed − prediction",
    "observed_minus_individual_prediction": "Residual, observed − individual prediction",
}


def residuals(result, *, x="time", standardized=None, output=None, time_unit=None):
    """Residuals against time or a prediction, with a zero reference line.

    Standardized residuals are the conditional residuals recorded by the fit
    (residual divided by the residual-error SD at the prediction); they are
    not CWRES or NPDE. Dashed lines mark ±1.96. Native population fits record
    no standardized residuals; their raw residuals are observed minus
    individual prediction. ``x`` is 'time', 'prediction' or
    'individual_prediction'.
    """
    _kind(result, ("fit", "population", "diagnostics", "native_fit"), "residuals")
    frame, output, unit, time = _fit_frame(result, output, time_unit, need_time=x == "time")
    available = frame["standardized_residual"].notna().any()
    standardized = available if standardized is None else standardized
    if standardized and not available:
        fail("this result records no standardized residuals; use standardized=False")
    censored = int(frame["censored"].sum())
    if censored:
        warnings.warn(f"{censored} rows below the LLOQ omitted", stacklevel=2)
    frame = frame[~frame["censored"]].copy()
    if x not in ("time", "prediction", "individual_prediction"):
        fail("x must be 'time', 'prediction' or 'individual_prediction'")
    if x == "individual_prediction" and not frame["individual_prediction"].notna().any():
        fail("this fit has no individual predictions")
    column = "standardized_residual" if standardized else "residual"
    frame = frame.assign(x=frame[x], y=frame[column])
    fig = ggplot(frame, aes("x", "y")) + geom_hline(yintercept=0, color=_c("muted"), size=style.line("reference"))
    if standardized:
        for bound in (-1.96, 1.96):
            fig += geom_hline(yintercept=bound, color=_c("muted"), size=style.line("reference"), linetype="dashed")
    fig += geom_point(color=_c("ink"), fill=_c("surface"), shape="o", size=style.point_size(), stroke=0.6)
    if x == "time":
        xlabel = _label("Time", time)
    elif x == "individual_prediction" or frame["individual_prediction"].isna().all():
        xlabel = _label(("Individual prediction" if x == "individual_prediction"
                         else "Model prediction") + f", {output}", unit)
    else:
        xlabel = _label(f"Population prediction, {output}", unit)
    residual_kind = _single(frame, "residual_kind", "residual kind")
    ylabel = ("Conditional standardized residual" if standardized
              else _label(_RESIDUAL_LABELS[residual_kind], unit))
    fig += labs(x=xlabel, y=ylabel)
    return style.size(fig + style.theme_style(), 80, 65)


def goodness_of_fit(result, *, output=None, log=False, time_unit=None):
    """Standard goodness-of-fit panels, labelled A–D.

    Population fits: observed vs population and individual predictions, and
    residuals vs time and vs the prediction the residual is computed against.
    Individual fits have three panels. Returns a plotnine composition.
    """
    _kind(result, ("fit", "population", "diagnostics", "native_fit"), "goodness_of_fit")
    frame, output, _, _ = _fit_frame(result, output, need_time=False)
    population = frame["individual_prediction"].notna().any()
    first = [observed_vs_predicted(result, prediction="population" if population else None,
                                   output=output, log=log)]
    if population:
        first.append(observed_vs_predicted(result, prediction="individual", output=output, log=log))
    against = "individual_prediction" if population else "prediction"
    second = [residuals(result, x="time", output=output, time_unit=time_unit),
              residuals(result, x=against, output=output)]
    panels = first + second
    titled = [p + labs(title=letter) for p, letter in zip(panels, "ABCD")]
    if population:
        composed = Stack([Beside(titled[:2]), Beside(titled[2:])])
        height = 165
    else:
        composed = Beside(titled)
        height = 70
    return style.size(composed, 180, height)


# Predictive checks ----------------------------------------------------------

def vpc(result, *, log=False, label=None):
    """Visual predictive check, or prediction-corrected VPC.

    Shaded boxes span each time bin and show the Monte Carlo confidence
    interval of each simulated percentile; points joined by lines are the
    observed percentiles at bin midpoints. Intervals are absent when any
    replicate failed; the simulated percentiles are then drawn as dashed
    lines and a warning is issued.
    """
    _kind(result, ("vpc",), "vpc")
    frame = to_frame(result, "bins")
    percentiles = sorted(frame["percentile"].unique())
    names = [_percentile_name(p) for p in percentiles]
    frame["band"] = frame["percentile"].map(dict(zip(percentiles, names)))
    frame["role"] = ["Median" if abs(p - 0.5) < 1e-12 else "Outer" for p in frame["percentile"]]
    interval = frame["lower"].notna().all() and frame["upper"].notna().all()
    observed = frame.dropna(subset=["observed"])
    observed = _positive(observed, ["observed"], log, "observed percentiles")
    level = f"{100 * result['confidence']:g}% CI"
    outer = " and ".join(n.replace(" percentile", "") for n in names if n != "median")
    simulated_keys = {"Median": f"Simulated median, {level}",
                      "Outer": f"Simulated {outer}, {level}"}
    observed_keys = {"Median": "Observed median", "Outer": f"Observed {outer}"}
    fig = ggplot()
    if interval:
        boxes = _positive(frame, ["lower", "upper"], log, "interval bounds")
        boxes = boxes.assign(key=boxes["role"].map(simulated_keys))
        fig += geom_rect(boxes, aes(xmin="start", xmax="end", ymin="lower", ymax="upper",
                                    fill="key"), alpha=style.fill_alpha())
        fig += scale_fill_manual(values={simulated_keys["Median"]: _cat(1),
                                         simulated_keys["Outer"]: _c("fill")})
    else:
        warnings.warn("the VPC has failed replicates, so it reports no intervals; "
                      "simulated percentiles are drawn without intervals", stacklevel=2)
        simulated = _positive(frame.dropna(subset=["simulated"]), ["simulated"], log,
                              "simulated percentiles")
        fig += geom_segment(simulated, aes(x="start", xend="end", y="simulated", yend="simulated"),
                            color=_cat(1), size=style.line("model"), linetype="dashed")
    observed = observed.assign(key=observed["role"].map(observed_keys))
    fig += geom_line(observed, aes("midpoint", "observed", group="band", linetype="key"),
                     color=_c("ink"), size=style.line("model"))
    fig += geom_point(observed, aes("midpoint", "observed"), color=_c("ink"), size=style.point_size() * 0.8)
    fig += scale_linetype_manual(values={observed_keys["Median"]: "solid",
                                         observed_keys["Outer"]: "dashed"})
    fig += guides(fill=guide_legend(ncol=1, order=1), linetype=guide_legend(ncol=1, order=2))
    strata = list(dict.fromkeys(frame["stratum"]))
    if len(strata) > 1 or strata != ["()"]:
        fig += facet_wrap("~stratum")
    if log:
        fig += scale_y_log10()
    name = label or result["output"]
    if result["prediction_corrected"]:
        name = f"Prediction-corrected {name}"
    fig += labs(x=_label("Time", result["time_unit"]),
                y=_label(name, result["unit"]) + (", log scale" if log else ""))
    return style.size(fig + style.theme_style(), 85, 85)


def vpc_blq(result):
    """Fraction of rows below the LLOQ in each time bin.

    The line joins the observed fractions; shaded boxes are the confidence
    intervals of the simulated fractions.
    """
    _kind(result, ("vpc",), "vpc_blq")
    if not result.get("lloq_coverage"):
        fail("the VPC was run without LLOQ values; no BLQ fractions are available")
    frame = to_frame(result, "blq")
    fig = ggplot(frame)
    if frame["lower"].notna().all():
        fig += geom_rect(aes(xmin="start", xmax="end", ymin="lower", ymax="upper"),
                         fill=_c("fill"), alpha=style.fill_alpha())
    else:
        warnings.warn("the VPC reports no BLQ intervals", stacklevel=2)
    fig += geom_line(aes("midpoint", "observed"), color=_c("ink"), size=style.line("model"))
    fig += geom_point(aes("midpoint", "observed"), color=_c("ink"), size=style.point_size())
    strata = list(dict.fromkeys(frame["stratum"]))
    if len(strata) > 1 or strata != ["()"]:
        fig += facet_wrap("~stratum")
    fig += scale_y_continuous(limits=(0, 1))
    fig += labs(x=_label("Time", result["time_unit"]),
                y="Fraction of observations below the LLOQ")
    return style.size(fig + style.theme_style(), 85, 65)


# Parameters and uncertainty -------------------------------------------------

def _estimates(estimates):
    found = kind(estimates)
    if found not in ("fit", "population", "native_fit"):
        fail("estimates must be a fit, population fit or native fit result")
    return to_frame(estimates, "parameters").set_index("parameter")


def parameters(result, *, estimates=None):
    """Parameter estimates with confidence intervals, relative to the estimate.

    Accepts a bootstrap, likelihood profile or native population fit with
    fixed-effect intervals. Each row is labelled with its estimate, interval
    and unit; the x axis is the ratio to the estimate on a log scale. A bound
    the method did not reach is reported as 'not reached'. For a bootstrap,
    ``estimates`` (the original fit) supplies the point estimates; otherwise
    the bootstrap median is used.
    """
    found = _kind(result, ("bootstrap", "profile", "native_fit", "fit", "population"), "parameters")
    frame = to_frame(result, "parameters")
    if estimates is not None:
        if found != "bootstrap":
            fail("estimates= applies only to bootstrap results")
        original = _estimates(estimates)
        for i, row in frame.iterrows():
            if row["parameter"] not in original.index:
                fail(f"estimates lack parameter {row['parameter']!r}")
            if original.loc[row["parameter"], "unit"] != row["unit"]:
                fail(f"estimate unit for {row['parameter']!r} differs from the bootstrap unit")
            frame.loc[i, "estimate"] = original.loc[row["parameter"], "estimate"]
    if frame[["lower", "upper"]].isna().all().all():
        fail("the result has no confidence intervals; use a bootstrap, likelihood profile "
             "or native population fit with fixed-effect intervals")
    if (frame["estimate"] <= 0).any() or (frame[["lower", "upper"]] <= 0).any().any():
        fail("relative intervals need positive estimates and bounds")

    def text(value):
        return "not reached" if pd.isna(value) else f"{value:.3g}"

    frame["label"] = [f"{_label(r.parameter, r.unit)}: {r.estimate:.3g} [{text(r.lower)}, {text(r.upper)}]"
                      for r in frame.itertuples()]
    frame["low"] = frame["lower"] / frame["estimate"]
    frame["high"] = frame["upper"] / frame["estimate"]
    frame["mid"] = 1.0
    frame = _ordered(frame, "label", list(frame["label"])[::-1])
    segments = frame.assign(low=frame["low"].fillna(1.0), high=frame["high"].fillna(1.0))
    confidence = frame["confidence"].dropna().unique()
    level = f"{100 * confidence[0]:g}% " if len(confidence) == 1 else ""
    fig = (ggplot(frame, aes(y="label"))
           + geom_vline(xintercept=1, color=_c("muted"), size=style.line("reference"), linetype="dashed")
           + geom_segment(segments, aes(x="low", xend="high", y="label", yend="label"),
                          color=_cat(0), size=style.line("interval"))
           + geom_point(aes(x="mid"), color=_c("ink"), size=style.point_size() * 1.2)
           + scale_x_log10()
           + labs(x=f"Ratio to estimate, {level}confidence interval (log scale)", y=""))
    return style.size(fig + style.theme_style() + theme(panel_grid_major_y=element_blank()),
                 85, 25 + 9 * len(frame))


def bootstrap(result, *, estimates=None, bins=20):
    """Distribution of converged bootstrap estimates, one panel per parameter.

    Dashed lines mark the percentile interval; the solid line marks the
    original estimate when ``estimates`` is given, otherwise the median.
    """
    _kind(result, ("bootstrap",), "bootstrap")
    if isinstance(bins, bool) or not isinstance(bins, int) or not 5 <= bins <= 100:
        fail("bins must be an integer in [5, 100]")
    frame = to_frame(result, "replicates")
    frame = frame[frame["status"] == "converged"]
    if frame.empty:
        fail("the bootstrap has no converged replicates")
    summary = to_frame(result, "parameters")
    if estimates is not None:
        original = _estimates(estimates)
        for i, row in summary.iterrows():
            if row["parameter"] not in original.index or original.loc[row["parameter"], "unit"] != row["unit"]:
                fail(f"estimates lack parameter {row['parameter']!r} in unit {row['unit']!r}")
            summary.loc[i, "estimate"] = original.loc[row["parameter"], "estimate"]
    labels = {r.parameter: _label(r.parameter, r.unit) for r in summary.itertuples()}
    frame = frame.assign(panel=frame["parameter"].map(labels))
    summary = summary.assign(panel=summary["parameter"].map(labels))
    order = list(labels.values())
    frame, summary = _ordered(frame, "panel", order), _ordered(summary, "panel", order)
    bounds = summary.melt(id_vars=["panel"], value_vars=["lower", "upper"]).dropna()
    fig = (ggplot(frame, aes("value"))
           + geom_histogram(bins=bins, fill=_c("fill"), color=_c("surface"), size=style.line("edge"))
           + geom_vline(summary, aes(xintercept="estimate"), color=_c("ink"), size=style.line("model"))
           + geom_vline(bounds, aes(xintercept="value"), color=_cat(1), size=style.line("model"),
                        linetype="dashed")
           + facet_wrap("~panel", scales="free")
           + labs(x="Bootstrap estimate", y="Converged replicates"))
    return style.size(fig + style.theme_style(), 180, 70)


def likelihood_profile(result):
    """Profile of the negative log-likelihood for one parameter.

    The dashed horizontal line is the likelihood-ratio threshold; dotted
    vertical lines mark the interval bounds the grid reached. Grid points that
    did not converge are omitted and counted in a warning.
    """
    _kind(result, ("profile",), "likelihood_profile")
    frame = to_frame(result, "profile")
    failed = int((frame["status"] != "converged").sum())
    if failed:
        warnings.warn(f"{failed} grid points did not converge and are omitted", stacklevel=2)
    frame = frame[frame["status"] == "converged"]
    if frame.empty:
        fail("no profile grid point converged")
    estimate = pd.DataFrame({"value": [result["estimate"]], "delta_nll": [0.0]})
    curve = pd.concat([frame[["value", "delta_nll"]], estimate]).sort_values("value")
    bounds = [b for b in result["interval"] if b is not None]
    fig = (ggplot(curve, aes("value", "delta_nll"))
           + geom_hline(yintercept=result["delta_nll_threshold"], color=_c("muted"), size=style.line("reference"),
                        linetype="dashed")
           + geom_line(color=_cat(0), size=style.line("model"))
           + geom_point(color=_c("ink"), size=style.point_size())
           + geom_point(estimate, aes("value", "delta_nll"), color=_cat(1), size=style.point_size() * 1.5))
    for bound in bounds:
        fig += geom_vline(xintercept=bound, color=_c("muted"), size=style.line("reference"), linetype="dotted")
    fig += labs(x=_label(result["parameter"], result["unit"]),
                y="Increase in negative log-likelihood")
    return style.size(fig + style.theme_style(), 85, 65)


# Regimens and exposure ------------------------------------------------------

def _comparison_output(result):
    comparisons = result.values() if collection_kind(result) == "regimens" else [result]
    outputs = {next(iter(c["regimens"].values()))["subject_metrics"][0]["output"]
               for c in comparisons}
    if len(outputs) != 1:
        fail("the comparisons simulate different outputs: " + ", ".join(sorted(outputs)))
    return outputs.pop()


def regimens(result, *, log=False, label=None, target=None):
    """Percentile bands of simulated profiles for each regimen.

    The line is the requested percentile nearest 0.5 (the median when
    requested), and the band spans the outermost requested percentiles.
    Concentration curves exclude measurement error.
    ``result`` may also be a dict of comparisons keyed by model, for example
    from two model variants with the same seed and regimens: models are then
    coloured and each regimen has its own panel. ``target`` draws a dashed
    reference line in the output unit.
    """
    found = _kind(result, ("regimens", "model_comparisons"), "regimens")
    frame = to_frame(result, "curves")
    output = _comparison_output(result)
    unit = _single(frame, "unit", "output unit")
    time = _single(frame, "time_unit", "time unit")
    percentiles = sorted(frame["percentile"].unique())
    if found == "model_comparisons":
        sets = {tuple(c["regimens"][next(iter(c["regimens"]))]["percentiles"]) for c in result.values()}
        if len(sets) != 1:
            fail("the comparisons report different percentiles")
    low, high = percentiles[0], percentiles[-1]
    middle = min(percentiles, key=lambda p: abs(p - 0.5))
    keys = (["model"] if found == "model_comparisons" else []) + ["regimen", "row", "time"]
    wide = frame.pivot_table(index=keys, columns="percentile", values="value",
                             observed=True).reset_index()
    wide = wide[keys].assign(low=wide[low], high=wide[high], middle=wide[middle])
    wide = _positive(wide, ["low", "middle", "high"], log)
    regimen_levels = list(dict.fromkeys(frame["regimen"]))
    wide = _ordered(wide, "regimen", regimen_levels).sort_values(keys)
    alpha = style.fill_alpha() * 0.6
    if found == "model_comparisons":
        models = list(frame["model"].cat.categories)
        if _many(len(models)):
            fail(f"at most {len(style.palette())} models can be overlaid")
        fig = (ggplot(wide, aes("time"))
               + geom_ribbon(aes(ymin="low", ymax="high", fill="model"), alpha=alpha)
               + geom_path(aes(y="middle", color="model"), size=style.line("model"))
               + style.scale_color_categorical() + style.scale_fill_categorical())
        if len(regimen_levels) > 1:
            fig += facet_wrap("~regimen")
    elif _many(len(regimen_levels)):
        fig = (ggplot(wide, aes("time"))
               + geom_ribbon(aes(ymin="low", ymax="high"), fill=_c("fill"), alpha=style.fill_alpha())
               + geom_path(aes(y="middle"), color=_cat(0), size=style.line("model"))
               + facet_wrap("~regimen"))
    else:
        fig = (ggplot(wide, aes("time"))
               + geom_ribbon(aes(ymin="low", ymax="high", fill="regimen"), alpha=alpha)
               + geom_path(aes(y="middle", color="regimen"), size=style.line("model"))
               + style.scale_color_categorical() + style.scale_fill_categorical())
    if target is not None:
        fig += geom_hline(yintercept=_finite(target, "target"), color=_c("ink"),
                          size=style.line("reference"), linetype="dashed")
    if log:
        fig += scale_y_log10()
    fig += labs(x=_label("Time", time),
                y=_label(label or output, unit) + (", log scale" if log else ""))
    panels = len(regimen_levels) if found == "model_comparisons" or _many(len(regimen_levels)) else 1
    if panels > 1:
        return style.size(fig + style.theme_style(), 180, 30 + 45 * math.ceil(panels / 3))
    return style.size(fig + style.theme_style(), 85, 70)


def _finite(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        fail(f"{name} must be a finite number")
    return float(value)


def target_attainment(result, metric, target, *, direction="below", confidence=0.95):
    """Probability of target attainment per regimen, with Wilson intervals.

    Counts the virtual subjects of a compare_regimens result whose ``metric``
    is at most (``direction='below'``) or at least (``'above'``) ``target``,
    in the metric's unit. Bars are Wilson score intervals at ``confidence``.
    A dict of comparisons keyed by model overlays the models on the same
    regimens. Regimens are listed in comparison order. The data are
    ``qspquest.tables.target_attainment`` with the same arguments.
    """
    found = _kind(result, ("regimens", "model_comparisons"), "target_attainment")
    table = _attainment_table(result, metric, target, direction=direction, confidence=confidence)
    output = _comparison_output(result)
    unit = table["unit"].iloc[0]
    levels = list(dict.fromkeys(table["regimen"]))[::-1]
    table = _ordered(table, "regimen", levels)
    sign = "≤" if direction == "below" else "≥"
    short = {"auc": "AUC", "cmax": "Cmax", "cmin": "Cmin", "tmax": "Tmax",
             "endpoint": "last value", "time_above_threshold": "time above threshold"}[metric]
    amount = f"{target:g}" if unit in (None, "", "1") else f"{target:g} {_unit(unit)}"
    if found == "model_comparisons":
        models = list(dict.fromkeys(table["model"]))
        if _many(len(models)):
            fail(f"at most {len(style.palette())} models can be overlaid")
        table = _ordered(table, "model", models)
        aesthetic = {"color": "model"}
        nudge = {m: ((len(models) - 1) / 2 - i) * 0.18 for i, m in enumerate(models)}
        table["y"] = table["regimen"].cat.codes + 1 + table["model"].map(nudge).astype(float)
    else:
        aesthetic = {}
        table["y"] = table["regimen"].cat.codes + 1.0
    fig = (ggplot(table, aes(y="y"))
           + geom_segment(aes(x="lower", xend="upper", yend="y", **aesthetic),
                          size=style.line("interval"),
                          **({} if aesthetic else {"color": _cat(0)}))
           + geom_point(aes(x="attainment", **aesthetic), size=style.point_size() * 1.3,
                        **({} if aesthetic else {"color": _c("ink")}))
           + scale_x_continuous(limits=(0, 1), breaks=[0, 0.25, 0.5, 0.75, 1],
                                labels=["0%", "25%", "50%", "75%", "100%"])
           + scale_y_continuous(breaks=list(range(1, len(levels) + 1)), labels=levels)
           + labs(x=f"Subjects with {short} of {output} {sign} {amount}", y=""))
    if aesthetic:
        fig += style.scale_color_categorical()
    return style.size(fig + style.theme_style() + theme(panel_grid_minor=element_blank()),
                      85, 30 + 14 * len(levels))


_METRICS = {"auc": "AUC over the sampled window", "cmax": "Cmax", "cmin": "Cmin",
            "tmax": "Tmax", "endpoint": "Value at the last sample",
            "time_above_threshold": "Time above threshold"}


def exposure(result, metric="auc", *, paired=True, log=False):
    """Per-subject exposure metric for each regimen.

    Points are virtual subjects; grey lines join the same subject across
    regimens when ``paired`` is true; the bar marks the median.
    """
    _kind(result, ("regimens",), "exposure")
    frame = to_frame(result, "subjects")
    if metric not in set(frame["metric"]):
        fail(f"metric must be one of {', '.join(dict.fromkeys(frame['metric']))}")
    frame = frame[frame["metric"] == metric]
    unit = _single(frame, "unit", "metric unit")
    frame = _positive(frame, ["value"], log)
    levels = list(result["regimens"])
    frame = _ordered(frame, "regimen", levels)
    frame["position"] = frame["regimen"].cat.codes + 1.0
    medians = frame.groupby("regimen", observed=True)["value"].median().reset_index()
    medians["position"] = medians["regimen"].cat.codes + 1.0
    fig = ggplot(frame, aes("position", "value"))
    if paired and len(levels) > 1:
        fig += geom_line(aes(group="subject"), color=_c("grid"), size=style.line("reference"))
    fig += geom_point(color=_c("ink"), fill=_c("surface"), shape="o", size=style.point_size(), stroke=0.5, alpha=0.8)
    fig += geom_segment(medians, aes(x="position - 0.25", xend="position + 0.25",
                                     y="value", yend="value"), color=_cat(1), size=style.line("interval"))
    fig += scale_x_continuous(breaks=list(range(1, len(levels) + 1)), labels=levels,
                              limits=(0.5, len(levels) + 0.5))
    if log:
        fig += scale_y_log10()
    output = next(iter(result["regimens"].values()))["subject_metrics"][0]["output"]
    fig += labs(x="", y=_label(f"{_METRICS[metric]}, {output}", unit) + (", log scale" if log else ""))
    return style.size(fig + style.theme_style() + theme(panel_grid_major_x=element_blank()), 85, 70)


def exposure_ratio(result, metrics=("auc", "cmax"), *, bounds=(0.8, 1.25)):
    """Paired ratio of each regimen to the reference regimen, per metric.

    For every virtual subject the metric under a regimen is divided by the
    same subject's metric under the reference. The point is the median ratio
    and the bar spans the comparison's outermost percentiles. ``bounds`` draws
    a shaded reference range (set None to omit).
    """
    _kind(result, ("regimens",), "exposure_ratio")
    if len(result["regimens"]) < 2:
        fail("exposure_ratio needs at least two regimens")
    frame = to_frame(result, "subjects")
    metrics = list(metrics)
    if not metrics or any(m not in ("auc", "cmax", "cmin", "endpoint") for m in metrics):
        fail("metrics must be chosen from auc, cmax, cmin and endpoint")
    reference = result["reference"]
    percentiles = next(iter(result["regimens"].values()))["percentiles"]
    low, high = min(percentiles), max(percentiles)
    rows = []
    for metric in metrics:
        subset = frame[frame["metric"] == metric]
        base = subset[subset["regimen"] == reference].set_index("subject")["value"]
        if (base <= 0).any():
            fail(f"reference {metric} must be positive for every subject")
        for label in result["regimens"]:
            if label == reference:
                continue
            values = subset[subset["regimen"] == label].set_index("subject")["value"]
            ratio = (values / base).dropna()
            rows.append({"regimen": label, "metric": _METRICS[metric], "median": ratio.median(),
                         "low": ratio.quantile(low), "high": ratio.quantile(high)})
    table = pd.DataFrame(rows)
    if (table[["median", "low", "high"]] <= 0).any().any():
        fail("ratios must be positive for a logarithmic axis")
    table = _ordered(table, "regimen", [r for r in result["regimens"] if r != reference][::-1])
    fig = ggplot(table, aes(y="regimen"))
    if bounds is not None:
        lower, upper = bounds
        if not 0 < lower < 1 < upper:
            fail("bounds must satisfy 0 < lower < 1 < upper")
        fig += geom_rect(aes(xmin=lower, xmax=upper, ymin=-math.inf, ymax=math.inf),
                         data=pd.DataFrame({"x": [0]}), inherit_aes=False, fill=_c("grid"), alpha=0.8)
    fig += geom_vline(xintercept=1, color=_c("muted"), size=style.line("reference"), linetype="dashed")
    fig += geom_segment(aes(x="low", xend="high", yend="regimen"), color=_cat(0),
                        size=style.line("interval"))
    fig += geom_point(aes(x="median"), color=_c("ink"), size=style.point_size() * 1.2)
    fig += facet_wrap("~metric", ncol=1)
    fig += scale_x_log10()
    fig += labs(x=f"Paired ratio to {reference} (log scale)", y="")
    return style.size(fig + style.theme_style() + theme(panel_grid_major_y=element_blank()),
                 85, 30 + 16 * len(metrics) * max(1, len(result["regimens"]) - 1))


# Population diagnostics -----------------------------------------------------

def random_effects(result, *, covariates=None, bins=15):
    """Distribution of individual random effects (η), one panel per effect.

    Panel titles report the η-variance shrinkage recorded by the fit. With
    ``covariates`` (a DataFrame with a ``subject`` column), η is plotted
    against each numeric covariate instead.
    """
    _kind(result, ("population",), "random_effects")
    frame = to_frame(result, "random_effects")
    labels = {}
    for effect, shrinkage in zip(frame["effect"], frame["shrinkage"]):
        text = "" if pd.isna(shrinkage) else f" (shrinkage {100 * shrinkage:.0f}%)"
        labels[effect] = f"η, {effect}{text}"
    frame["panel"] = frame["effect"].map(labels)
    frame = _ordered(frame, "panel", list(dict.fromkeys(labels.values())))
    if covariates is None:
        fig = (ggplot(frame, aes("eta"))
               + geom_histogram(bins=bins, fill=_c("fill"), color=_c("surface"), size=style.line("edge"))
               + geom_vline(xintercept=0, color=_c("muted"), size=style.line("reference"), linetype="dashed")
               + facet_wrap("~panel", scales="free_x")
               + labs(x="Individual random effect, η (log-parameter scale)", y="Subjects"))
        return style.size(fig + style.theme_style(), 180, 65)
    if not isinstance(covariates, pd.DataFrame) or "subject" not in covariates:
        fail("covariates must be a DataFrame with a subject column")
    numeric = [c for c in covariates.columns if c != "subject"
               and pd.api.types.is_numeric_dtype(covariates[c])]
    if not numeric:
        fail("covariates has no numeric columns")
    table = covariates.assign(subject=covariates["subject"].astype(str))
    missing = set(frame["subject"]) - set(table["subject"])
    if missing:
        fail("covariates lack subjects: " + ", ".join(sorted(missing)))
    long = frame.merge(table.melt(id_vars="subject", value_vars=numeric,
                                  var_name="covariate", value_name="value"), on="subject")
    fig = (ggplot(long, aes("value", "eta"))
           + geom_hline(yintercept=0, color=_c("muted"), size=style.line("reference"), linetype="dashed")
           + geom_point(color=_c("ink"), fill=_c("surface"), shape="o", size=style.point_size(), stroke=0.6)
           + facet_wrap("~panel + covariate", scales="free")
           + labs(x="Covariate value", y="η (log-parameter scale)"))
    return style.size(fig + style.theme_style(), 180, 60 * len(numeric))


def trace(result):
    """Parameter estimates over SAEM iterations or optimizer simulations."""
    found = _kind(result, ("fit", "population"), "trace")
    frame = to_frame(result, "trace")
    if frame.empty:
        fail("the fit records no trace")
    frame["panel"] = [_label(p, u) for p, u in zip(frame["parameter"], frame["unit"])]
    frame = _ordered(frame, "panel", list(dict.fromkeys(frame["panel"])))
    final = frame.groupby("panel", observed=True).tail(1)
    fig = (ggplot(frame, aes("step", "value"))
           + geom_line(color=_cat(0), size=style.line("model"))
           + geom_hline(final, aes(yintercept="value"), color=_c("muted"), size=style.line("reference"), linetype="dashed")
           + facet_wrap("~panel", scales="free_y")
           + labs(x="Iteration" if found == "population" else "Simulation", y="Estimate"))
    return style.size(fig + style.theme_style(), 180, 60)


# Sensitivity ----------------------------------------------------------------

def morris(result, *, label=None):
    """Morris screening: μ* (mean absolute elementary effect) against σ.

    Points far from the origin along μ* have a large overall effect; a large
    σ relative to μ* indicates interaction or nonlinearity.
    """
    _kind(result, ("morris",), "morris")
    frame = to_frame(result, "effects")
    unit = _single(frame, "unit", "effect unit")
    top = max(frame["mu_star"].max(), frame["sigma"].max()) * 1.1 or 1.0
    name = f", {label}" if label else ""
    fig = (ggplot(frame, aes("mu_star", "sigma"))
           + geom_abline(intercept=0, slope=1, color=_c("grid"), size=style.line("reference"))
           + geom_point(color=_cat(0), size=style.point_size() * 1.6)
           + geom_text(aes(label="parameter"), color=_c("ink"), size=style.type_sizes()[1], ha="center",
                       family=style.font(),
                       va="bottom", nudge_y=top * 0.03)
           + scale_x_continuous(limits=(0, top))
           + scale_y_continuous(limits=(0, top))
           + coord_equal()
           + labs(x=_label(f"μ*, mean absolute elementary effect{name}", unit),
                  y=_label(f"σ, SD of elementary effects{name}", unit)))
    return style.size(fig + style.theme_style(), 80, 80)


def _metric_values(frame, metric):
    groups = frame.groupby("point", sort=False)
    if metric == "max":
        return groups["value"].max()
    if metric == "min":
        return groups["value"].min()
    if metric == "last":
        return groups["value"].last()
    if metric == "auc":
        def area(part):
            t, y = part["time"].to_numpy(), part["value"].to_numpy()
            return float(((t[1:] - t[:-1]) * (y[1:] + y[:-1]) / 2).sum())
        return groups[["time", "value"]].apply(area)
    fail("metric must be 'max', 'min', 'last' or 'auc'")


_SCAN_METRICS = {"max": "Maximum", "min": "Minimum", "last": "Last sampled value",
                 "auc": "AUC over the sampled window"}


def scan(result, output=None, *, metric="max", label=None, log=None):
    """Parameter scan: trajectories coloured by one varied parameter, or a heat map.

    With one varied parameter, each point's trajectory is drawn and coloured
    on a sequential scale. With two, each tile shows ``metric`` ('max', 'min',
    'last' or 'auc' over the sampled window) for one combination. ``log``
    selects logarithmic parameter axes; by default they are logarithmic when
    the scanned range spans a factor of 10 or more.
    """
    _kind(result, ("scan",), "scan")
    values = to_frame(result, "parameters")
    trajectory = to_frame(result, "trajectory")
    if output is None:
        outputs = list(dict.fromkeys(trajectory["output"]))
        if len(outputs) > 1:
            fail(f"the scan has outputs {', '.join(outputs)}; choose one with output=")
        output = outputs[0]
    if output not in set(trajectory["output"]):
        fail(f"unknown output {output!r}")
    trajectory = trajectory[trajectory["output"] == output]
    unit = _single(trajectory, "unit", "output unit")
    time = _single(trajectory, "time_unit", "time unit")
    varied = [p for p, part in values.groupby("parameter", sort=False) if part["value"].nunique() > 1]
    if not varied:
        fail("no parameter varies across the scan points")
    if len(varied) > 2:
        fail("the scan varies more than two parameters: " + ", ".join(varied))
    units = {p: _single(values[values["parameter"] == p], "unit", f"{p} unit") for p in varied}
    wide = values[values["parameter"].isin(varied)].pivot(index="point", columns="parameter",
                                                          values="value")

    def logarithmic(name):
        column = wide[name]
        automatic = column.min() > 0 and column.max() / column.min() >= 10
        if log and column.min() <= 0:
            fail(f"{name} has non-positive values; use log=False")
        return automatic if log is None else bool(log)

    if len(varied) == 1:
        name = varied[0]
        frame = trajectory.merge(wide.reset_index(), on="point")
        fig = (ggplot(frame, aes("time", "value", group="point", color=name))
               + geom_path(size=style.line("model"))
               + style.scale_color_sequential(
                   name=_label(name, units[name]),
                   trans="log10" if logarithmic(name) else "identity")
               + labs(x=_label("Time", time), y=_label(label or output, unit)))
        return style.size(fig + style.theme_style() + theme(legend_position="right",
                                            legend_title=element_text(color=_c("ink"))), 85, 65)
    x, y = varied
    summary = _metric_values(trajectory, metric).rename("metric").reset_index()
    frame = wide.reset_index().merge(summary, on="point")
    metric_unit = f"{unit} * {time}" if metric == "auc" else unit
    fig = (ggplot(frame, aes(x, y, fill="metric"))
           + geom_tile(color=_c("surface"), size=style.line("edge"))
           + style.scale_fill_sequential(name=_label(f"{_SCAN_METRICS[metric]}, {label or output}",
                                                         metric_unit))
           + labs(x=_label(x, units[x]), y=_label(y, units[y])))
    if logarithmic(x):
        fig += scale_x_log10()
    if logarithmic(y):
        fig += scale_y_log10()
    return style.size(fig + style.theme_style() + theme(legend_position="right",
                                        legend_title=element_text(color=_c("ink"))), 85, 70)


def steady_state(result):
    """Pre-cycle amounts at periodic steady state, one row per state.

    Amounts are the states immediately before the cycle's start-time events.
    States with different units are drawn in separate panels.
    """
    _kind(result, ("steady_state",), "steady_state")
    frame = to_frame(result, "states")
    frame["panel"] = "Amount (" + frame["unit"].map(_unit) + ")"
    frame = _ordered(frame, "state", list(frame["state"])[::-1])
    fig = (ggplot(frame, aes(y="state"))
           + geom_segment(aes(x=0, xend="amount", yend="state"), color=_c("grid"), size=style.line("interval"))
           + geom_point(aes(x="amount"), color=_cat(0), size=style.point_size() * 1.6)
           + labs(x="Pre-cycle amount at periodic steady state", y=""))
    if frame["unit"].nunique() > 1:
        fig += facet_wrap("~panel", scales="free_x")
    else:
        fig += labs(x=_label("Pre-cycle amount at periodic steady state", frame["unit"].iloc[0]))
    return style.size(fig + style.theme_style() + theme(panel_grid_major_y=element_blank()),
                 85, 25 + 9 * len(frame))
