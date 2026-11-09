"""Figures and tables build from native results, carry units and reject bad input.

The checks inspect labels, data and file structure; they never compare images.
"""

import copy
import json
import math
import re
from pathlib import Path
import subprocess
import sys
import warnings

import pytest

pd = pytest.importorskip("pandas")
pytest.importorskip("plotnine")
import matplotlib

matplotlib.use("Agg")
from matplotlib import pyplot
from plotnine import ggplot
from plotnine.composition import Compose

import qspquest as qq
from qspquest import QSPQuestError

ROOT = Path(__file__).resolve().parents[3]
CONFORMANCE = ROOT / "conformance"
ERROR = {"model": "normal", "additive": 0.2, "proportional": 0.1}
PARAMETERS = {
    "cl": {"initial": 2.0, "lower": 0.1, "upper": 10.0, "unit": "L/h"},
    "vc": {"initial": 30.0, "lower": 5.0, "upper": 100.0, "unit": "L"},
    "ka": {"initial": 1.0, "lower": 0.1, "upper": 6.0, "unit": "1/h"},
}
OMEGA = [[0.04, 0, 0], [0, 0.04, 0], [0, 0, 0.04]]


def _subjects(model, count=6):
    """Synthetic oral-PK observations from the analytic solution with fixed noise."""
    times = [0.25, 0.5, 1, 2, 4, 8, 12, 24]
    noise = [0.05, -0.08, 0.03, 0.06, -0.04, 0.02, -0.06, 0.04]
    subjects = []
    for index in range(count):
        cl, vc, ka = 1.8 * math.exp(0.1 * (index - 2.5)), 30.0 * math.exp(0.05 * (index % 3 - 1)), 1.2
        rate = cl / vc
        values = [180 / vc * ka / (ka - rate) * (math.exp(-rate * t) - math.exp(-ka * t))
                  * (1 + e) for t, e in zip(times, noise[index:] + noise[:index])]
        run = qq.run_request(end=24, times=times, administrations=[qq.bolus("depot", 180, "mg")],
                             solver="analytic_linear_pk")
        observations = [{"row": i, "output": "cp", "value": qq.quantity(v, "mg/L")}
                        for i, v in enumerate(values)]
        subjects.append({"id": str(index + 1), "run": run, "observations": observations})
    return subjects


def _native_population_fit():
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from test_population_dataset import prepare

    model = json.loads((CONFORMANCE / "models/synthetic-one-compartment.json").read_text())
    compiled = qq.CompiledSensitivities(model, ["cl"])
    rows = []
    for index, eta in enumerate((-0.22, -0.08, 0.08, 0.22)):
        rows.append({"ID": f"S{index + 1}", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6})
        for time in (1, 2, 4):
            dv = 6 / 4.5 * math.exp(-0.6 * math.exp(eta) * time / 4.5)
            rows.append({"ID": f"S{index + 1}", "TIME": time, "EVID": 0, "CMT": 2, "DV": dv})
    return compiled.fit_population_data(prepare(rows))


@pytest.fixture(scope="module")
def results():
    model = qq.CompiledModel((ROOT / "examples/synthetic/absorption.qsp").read_text())
    subjects = _subjects(model)
    run = model.run(qq.run_request(end=24, times=[0, 0.5, 1, 2, 4, 8, 12, 24],
                                   administrations=[qq.bolus("depot", 180, "mg")],
                                   solver="analytic_linear_pk"))
    fit = model.fit_data(subjects[0]["run"], subjects[0]["observations"], PARAMETERS, ERROR,
                         method="derivative_free", max_iterations=200, max_simulations=5000)
    population = model.fit_population(subjects, PARAMETERS, OMEGA, ERROR, max_iterations=40,
                                      burn_in=10, max_simulations=100000, seed=3)
    estimates = population["parameters"]
    vpc = model.visual_predictive_check(subjects, estimates, population["omega"],
                                        population["error"], replicates=10, seed=1)
    pcvpc = model.visual_predictive_check(subjects, estimates, population["omega"],
                                          population["error"], replicates=10, seed=1,
                                          prediction_corrected=True)
    profile = model.profile_likelihood(subjects[0]["run"], subjects[0]["observations"],
                                       PARAMETERS, ERROR, "cl", [1.2, 1.6, 2.4, 3.2],
                                       max_simulations=50000)
    twice = copy.deepcopy(subjects[0]["run"])
    dose = twice["regimen"]["administrations"][0]
    dose["amount"]["value"] = 90
    second = copy.deepcopy(dose)
    second["time"]["value"] = 12
    twice["regimen"]["administrations"].append(second)
    comparison = model.compare_regimens({"180 mg once": subjects[0]["run"], "90 mg twice": twice},
                                        estimates, population["omega"], "cp", subjects=12, seed=4)
    one = qq.CompiledModel((CONFORMANCE / "models/synthetic-one-compartment.qsp").read_text())
    morris = one.morris(json.loads((CONFORMANCE / "requests/synthetic-morris.json").read_text()))
    scan = one.scan(json.loads((CONFORMANCE / "requests/synthetic-scan.json").read_text()))
    grid = json.loads((CONFORMANCE / "requests/synthetic-scan.json").read_text())
    grid["points"] = [{"id": f"p{i}{j}", "parameters": {"cl": qq.quantity(cl, "L/h"),
                                                         "v": qq.quantity(v, "L")}}
                      for i, cl in enumerate((0.3, 0.6)) for j, v in enumerate((3.0, 6.0))]
    grid["total_budgets"] = {"solver_callbacks": 4000000, "events": 400, "output_values": 40000}
    scan_grid = one.scan(grid)
    linear = qq.CompiledModel(json.loads((CONFORMANCE / "models/synthetic-linear-pk.json").read_text()))
    steady = linear.periodic_steady_state(
        json.loads((CONFORMANCE / "requests/synthetic-linear-pk-steady.json").read_text()))
    bootstrap = {
        "schema": "qspquest.bootstrap/v0.1", "method": "subject_percentile_bootstrap",
        "status": "complete", "confidence": 0.9, "successful_replicates": 3,
        "failed_replicates": 1,
        "replicates": [{"replicate": i, "status": s, "parameters": None if s != "converged" else {
            "cl": {"value": v, "unit": "L/h"}, "vc": {"value": 30 + v, "unit": "L"}}}
            for i, (s, v) in enumerate([("converged", 1.7), ("converged", 1.9),
                                        ("converged", 2.1), ("iteration_limit", 0)])],
        "intervals": {"cl": {"unit": "L/h", "bounds": [1.72, 2.08]},
                      "vc": {"unit": "L", "bounds": [31.72, 32.08]}},
    }
    return {
        "run": run, "fit": fit, "population": population, "diagnostics": qq.fit_diagnostics(fit),
        "vpc": vpc, "pcvpc": pcvpc, "profile": profile, "comparison": comparison,
        "exposure": qq.exposure_summary(run, "cp"), "morris": morris, "scan": scan,
        "scan_grid": scan_grid, "steady": steady, "bootstrap": bootstrap,
        "native": _native_population_fit(),
    }


def _plots(figure):
    if isinstance(figure, ggplot):
        return [figure]
    return [plot for item in figure for plot in _plots(item)]


def _built(figure):
    assert isinstance(figure, (ggplot, Compose))
    pyplot.close(figure.draw())
    return figure


def _labels(figure):
    return figure.labels.x, figure.labels.y


def test_import_does_not_load_plotnine():
    code = "import sys, qspquest; assert 'plotnine' not in sys.modules; qspquest.plot; assert 'plotnine' in sys.modules"
    subprocess.run([sys.executable, "-c", code], check=True)


def test_profiles_take_labels_and_units_from_results(results):
    figure = _built(qq.plot.profile(results["run"]))
    assert _labels(figure) == ("Time (h)", "cp (mg/L)")
    named = qq.plot.profile(results["run"], label="Plasma concentration")
    assert named.labels.y == "Plasma concentration (mg/L)"
    with pytest.warns(UserWarning, match="1 non-positive"):
        logged = _built(qq.plot.profile(results["run"], log=True))
    assert logged.labels.y == "cp (mg/L), log scale"
    _built(qq.plot.profile({"A": results["run"], "B": results["run"]}))
    many = _built(qq.plot.profile({str(i): results["run"] for i in range(4)}))
    assert type(many.facet).__name__ == "facet_wrap"
    _built(qq.plot.profile(results["scan"]))
    for key in ("fit", "population", "diagnostics", "native"):
        figure = _built(qq.plot.profile(results[key]))
        assert figure.labels.x == "Time (h)"
        assert figure.labels.y == "cp (mg/L)"
    _built(qq.plot.profile(results["population"], subjects=["1", "2"]))
    observed = pd.DataFrame({"time": [1.0, 2.0], "value": [2.0, 3.0], "unit": "mg/L",
                             "time_unit": "h"})
    _built(qq.plot.profile(results["run"], observed=observed))
    # Duplicate pre/post rows are kept in time order and joined by a vertical step.
    frame = qq.to_frame(results["scan"])
    first = frame[frame["point"] == frame["point"].iloc[0]]
    assert list(first["side"][:2]) == ["pre", "post"] and first["time"].iloc[0] == first["time"].iloc[1]


def test_profile_rejects_mismatched_or_ambiguous_input(results):
    with pytest.raises(QSPQuestError, match="likelihood_profile"):
        qq.plot.profile(results["profile"])
    with pytest.raises(QSPQuestError, match="unknown output"):
        qq.plot.profile(results["run"], output="missing")
    wrong = pd.DataFrame({"time": [1.0], "value": [2.0], "unit": "ug/L", "time_unit": "h"})
    with pytest.raises(QSPQuestError, match="output unit"):
        qq.plot.profile(results["run"], observed=wrong)
    with pytest.raises(QSPQuestError, match="missing columns"):
        qq.plot.profile(results["run"], observed=pd.DataFrame({"time": [1.0]}))
    with pytest.raises(QSPQuestError, match="unknown subjects"):
        qq.plot.profile(results["population"], subjects=["99"])
    days = copy.deepcopy(results["run"])
    days["time_unit"] = "d"
    with pytest.raises(QSPQuestError, match="time unit"):
        qq.plot.profile({"A": results["run"], "B": days})
    undated = copy.deepcopy(results["fit"])
    for row in undated["fitted_observations"]:
        del row["time_unit"]
    with pytest.raises(QSPQuestError, match="time_unit="):
        qq.plot.profile(undated)
    assert qq.plot.profile(undated, time_unit="h").labels.x == "Time (h)"
    with pytest.raises(QSPQuestError, match="differs"):
        qq.plot.profile(results["fit"], time_unit="d")
    with pytest.raises(QSPQuestError, match="unsupported result"):
        qq.plot.profile({"schema": "qspquest.unknown/v0.1"})


def test_goodness_of_fit_panels_and_residual_labels(results):
    composed = _built(qq.plot.goodness_of_fit(results["population"]))
    plots = _plots(composed)
    assert [p.labels.title for p in plots] == ["A", "B", "C", "D"]
    assert plots[0].labels.x == "Population prediction, cp (mg/L)"
    assert plots[1].labels.x == "Individual prediction, cp (mg/L)"
    assert plots[0].labels.y == "Observed cp (mg/L)"
    assert plots[2].labels == plots[2].labels and plots[2].labels.x == "Time (h)"
    assert plots[2].labels.y == "Conditional standardized residual"
    individual = _plots(_built(qq.plot.goodness_of_fit(results["fit"])))
    assert len(individual) == 3 and individual[0].labels.x == "Model prediction, cp (mg/L)"
    native = _plots(_built(qq.plot.goodness_of_fit(results["native"])))
    assert native[2].labels.y == "Residual, observed − individual prediction (mg/L)"
    _built(qq.plot.goodness_of_fit(results["diagnostics"]))
    with pytest.raises(QSPQuestError, match="no standardized residuals"):
        qq.plot.residuals(results["native"], standardized=True)
    with pytest.raises(QSPQuestError, match="prediction='model'"):
        qq.plot.observed_vs_predicted(results["fit"], prediction="population")
    with pytest.raises(QSPQuestError, match="'population' or 'individual'"):
        qq.plot.observed_vs_predicted(results["population"], prediction="typical")
    with pytest.raises(QSPQuestError, match="x must be"):
        qq.plot.residuals(results["population"], x="covariate")
    with pytest.raises(QSPQuestError, match="accepts"):
        qq.plot.goodness_of_fit(results["run"])
    raw = _built(qq.plot.residuals(results["population"], standardized=False, x="prediction"))
    assert raw.labels.y == "Residual, observed − individual prediction (mg/L)"
    assert raw.labels.x == "Population prediction, cp (mg/L)"


def test_predictive_checks(results):
    figure = _built(qq.plot.vpc(results["vpc"]))
    assert _labels(figure) == ("Time (h)", "cp (mg/L)")
    corrected = _built(qq.plot.vpc(results["pcvpc"], log=True))
    assert corrected.labels.y == "Prediction-corrected cp (mg/L), log scale"
    failed = copy.deepcopy(results["vpc"])
    for record in failed["bins"]:
        for quantile in record["quantiles"]:
            quantile["confidence_interval"] = None
    with pytest.warns(UserWarning, match="no intervals"):
        _built(qq.plot.vpc(failed))
    with pytest.raises(QSPQuestError, match="without LLOQ"):
        qq.plot.vpc_blq(results["vpc"])
    censored = copy.deepcopy(results["vpc"])
    censored["lloq_coverage"] = censored["total_observations"]
    blq = _built(qq.plot.vpc_blq(censored))
    assert blq.labels.y == "Fraction of observations below the LLOQ"
    with pytest.raises(QSPQuestError, match="accepts vpc"):
        qq.plot.vpc(results["profile"])


def test_parameter_uncertainty(results):
    figure = _built(qq.plot.parameters(results["profile"]))
    labels = list(figure.data["label"])
    assert labels[0].startswith("cl (L/h): ")
    assert "95% confidence interval" in figure.labels.x
    boot = _built(qq.plot.parameters(results["bootstrap"]))
    assert "90% confidence interval" in boot.labels.x
    _built(qq.plot.parameters(results["bootstrap"], estimates=results["fit"]))
    with pytest.raises(QSPQuestError, match="no confidence intervals"):
        qq.plot.parameters(results["native"])
    native = copy.deepcopy(results["native"])
    native["result"]["population"]["fixed_effect_intervals"] = {
        "method": "positive_fixed_effect_log_wald_delta_v1", "confidence_level": 0.95,
        "critical_value": 1.96, "intervals": [{
            "name": "cl", "estimate": 0.6, "lower": 0.5, "upper": 0.72, "unit": "L/h",
            "log_coordinate_standard_error": 0.09, "source_standard_error": 0.054}]}
    figure = _built(qq.plot.parameters(native))
    assert list(figure.data["label"]) == ["cl (L/h): 0.6 [0.5, 0.72]"]
    native["result"]["population"]["fixed_effect_intervals"]["intervals"][0]["unit"] = "mL/min"
    with pytest.raises(QSPQuestError, match="unit"):
        qq.plot.parameters(native)
    with pytest.raises(QSPQuestError, match="no confidence intervals"):
        qq.plot.parameters(results["fit"])
    with pytest.raises(QSPQuestError, match="only to bootstrap"):
        qq.plot.parameters(results["profile"], estimates=results["fit"])
    histogram = _built(qq.plot.bootstrap(results["bootstrap"], estimates=results["fit"]))
    assert histogram.labels.x == "Bootstrap estimate"
    assert set(histogram.data["panel"]) == {"cl (L/h)", "vc (L)"}
    assert len(histogram.data) == 6  # converged replicates only
    with pytest.raises(QSPQuestError, match="bins"):
        qq.plot.bootstrap(results["bootstrap"], bins=2)
    profile = _built(qq.plot.likelihood_profile(results["profile"]))
    assert _labels(profile) == ("cl (L/h)", "Increase in negative log-likelihood")
    unit_mismatch = copy.deepcopy(results["fit"])
    unit_mismatch["parameters"]["cl"]["unit"] = "mL/min"
    with pytest.raises(QSPQuestError, match="unit"):
        qq.plot.parameters(results["bootstrap"], estimates=unit_mismatch)


def test_regimens_and_exposure(results):
    comparison = results["comparison"]
    figure = _built(qq.plot.regimens(comparison))
    assert _labels(figure) == ("Time (h)", "cp (mg/L)")
    assert figure.labels.caption is None  # journals keep captions outside the artwork
    auc = _built(qq.plot.exposure(comparison))
    assert auc.labels.y == "AUC over the sampled window, cp (mg/L·h)"
    assert _built(qq.plot.exposure(comparison, "tmax")).labels.y == "Tmax, cp (h)"
    ratio = _built(qq.plot.exposure_ratio(comparison))
    assert ratio.labels.x == "Paired ratio to 180 mg once (log scale)"
    medians = ratio.data.set_index("metric")["median"]
    subjects = qq.to_frame(comparison, "subjects")
    auc_rows = subjects[subjects["metric"] == "auc"].pivot(index="subject", columns="regimen",
                                                          values="value")
    expected = (auc_rows["90 mg twice"] / auc_rows["180 mg once"]).median()
    assert medians["AUC over the sampled window"] == pytest.approx(expected)
    with pytest.raises(QSPQuestError, match="metric must be"):
        qq.plot.exposure(comparison, "half_life")
    with pytest.raises(QSPQuestError, match="metrics must"):
        qq.plot.exposure_ratio(comparison, metrics=("tmax",))
    single = copy.deepcopy(comparison)
    single["regimens"] = {"180 mg once": single["regimens"]["180 mg once"]}
    with pytest.raises(QSPQuestError, match="at least two"):
        qq.plot.exposure_ratio(single)


def test_population_diagnostics(results):
    population = results["population"]
    effects = _built(qq.plot.random_effects(population))
    assert effects.labels.x.startswith("Individual random effect")
    assert any("shrinkage" in panel for panel in effects.data["panel"])
    covariates = pd.DataFrame({"subject": ["1", "2", "3", "4", "5", "6"],
                               "WT": [60, 65, 70, 75, 80, 85]})
    _built(qq.plot.random_effects(population, covariates=covariates))
    with pytest.raises(QSPQuestError, match="lack subjects"):
        qq.plot.random_effects(population, covariates=covariates.iloc[:2])
    trace = _built(qq.plot.trace(population))
    assert trace.labels.x == "Iteration"
    assert set(trace.data["panel"]) == {"cl (L/h)", "vc (L)", "ka (1/h)"}
    assert _built(qq.plot.trace(results["fit"])).labels.x == "Simulation"


def test_sensitivity_and_steady_state(results):
    morris = _built(qq.plot.morris(results["morris"]))
    assert morris.labels.x == "μ*, mean absolute elementary effect (mg/L)"
    assert morris.labels.y == "σ, SD of elementary effects (mg/L)"
    lines = _built(qq.plot.scan(results["scan"]))
    assert _labels(lines) == ("Time (h)", "cp (mg/L)")
    heat = _built(qq.plot.scan(results["scan_grid"], metric="auc"))
    assert _labels(heat) == ("cl (L/h)", "v (L)")
    with pytest.raises(QSPQuestError, match="metric must be"):
        qq.plot.scan(results["scan_grid"], metric="median")
    flat = copy.deepcopy(results["scan"])
    for point in flat["results"]:
        point["parameters"]["cl"]["value"] = 0.3
    with pytest.raises(QSPQuestError, match="no parameter varies"):
        qq.plot.scan(flat)
    steady = _built(qq.plot.steady_state(results["steady"]))
    assert steady.labels.x == "Pre-cycle amount at periodic steady state (mg)"


def test_tables_cover_every_result_with_units(results):
    expected = {
        "run": ("trajectory", {"time_unit", "unit"}),
        "fit": ("observations", {"time_unit", "unit"}),
        "population": ("observations", {"time_unit", "unit"}),
        "diagnostics": ("observations", {"time_unit", "unit"}),
        "native": ("observations", {"time_unit", "unit"}),
        "vpc": ("bins", {"time_unit", "unit"}),
        "profile": ("profile", {"unit"}),
        "bootstrap": ("replicates", {"unit"}),
        "comparison": ("curves", {"time_unit", "unit"}),
        "exposure": ("metrics", {"unit"}),
        "morris": ("effects", {"unit"}),
        "scan": ("trajectory", {"time_unit", "unit"}),
        "steady": ("states", {"unit", "residual_unit"}),
    }
    for key, (default, units) in expected.items():
        names = qq.tables.tables(results[key])
        assert names[0] == default
        for name in names:
            frame = qq.to_frame(results[key], name)
            assert not frame.empty, (key, name)
            assert any(column.endswith("unit") for column in frame.columns), (key, name)
        assert units <= set(qq.to_frame(results[key]).columns), key
    with pytest.raises(QSPQuestError, match="tables"):
        qq.to_frame(results["run"], "parameters")
    parameters = qq.to_frame(results["population"], "parameters")
    assert list(parameters["unit"]) == ["L/h", "L", "1/h"]
    subjects = qq.to_frame(results["comparison"], "subjects")
    units = dict(zip(subjects["metric"], subjects["unit"]))
    assert units == {"auc": "mg/L * h", "cmax": "mg/L", "tmax": "h", "cmin": "mg/L",
                     "endpoint": "mg/L"}


def test_analysis_result_to_frame_keeps_default_rows(results):
    fit = qq.AnalysisResult(results["fit"])
    assert fit.to_frame().equals(pd.DataFrame(fit.rows()))
    assert list(fit.to_frame("parameters")["parameter"]) == ["cl", "vc", "ka"]
    vpc = qq.AnalysisResult(results["vpc"])
    assert "percentile" in vpc.to_frame().columns


def test_style_layer_reads_every_colour_from_the_style_file():
    style = qq.plot.style
    definitions = json.loads((ROOT / "spec/style/figure-style.json").read_text())
    package = Path(style.__file__).with_name("figure-style.json")
    assert json.loads(package.read_text()) == definitions  # packaged copy matches the source
    subprocess.run([sys.executable, str(ROOT / "scripts/generate_figure_style.py"), "--check"],
                   check=True, capture_output=True)
    plot_dir = Path(style.__file__).parent
    for source in (plot_dir / "pkpd.py", plot_dir / "style.py", plot_dir / "__init__.py",
                   ROOT / "bindings/r/R/plot.R", ROOT / "bindings/r/R/style.R"):
        assert not re.search(r"#[0-9A-Fa-f]{6}\b|\"white\"", source.read_text()), source
    assert qq.plot.palette() == definitions["modes"]["light"]["categorical"]
    assert qq.plot.sequential() == definitions["modes"]["light"]["sequential"]
    assert qq.plot.color("ink") == definitions["modes"]["light"]["ink"]
    with pytest.raises(qq.plot.StyleError, match="facet"):
        qq.plot.palette(4)
    with pytest.raises(qq.plot.StyleError, match="colour name"):
        qq.plot.color("accent")
    with pytest.raises(qq.plot.StyleError, match="journal"):
        qq.plot.theme_qspquest(journal="unknown")
    with pytest.raises(qq.plot.StyleError, match="type sizes"):
        qq.plot.theme_qspquest(base_size=6, small_size=8)
    nature = qq.plot.theme_qspquest(journal="nature")
    assert nature.themeables["axis_text"].properties["size"] == 6
    assert qq.plot.journals()["jpkpd"]["single_mm"] == 84


def test_dark_mode_uses_dark_colours(results):
    definitions = qq.plot.definitions()["modes"]["dark"]
    try:
        qq.plot.set_mode("dark")
        assert qq.plot.palette() == definitions["categorical"]
        figure = _built(qq.plot.regimens(results["comparison"]))
        background = figure.theme.themeables["plot_background"].properties["facecolor"]
        assert background == definitions["surface"]
    finally:
        qq.plot.set_mode("light")
    with pytest.raises(qq.plot.StyleError, match="mode"):
        qq.plot.set_mode("sepia")


def test_save_writes_embedded_fonts_and_no_timestamps(results, tmp_path):
    figure = qq.plot.profile(results["run"])
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        paths = qq.plot.save(figure, tmp_path / "figure-1")
    assert [p.suffix for p in paths] == [".pdf", ".svg", ".png"]
    pdf = paths[0].read_bytes()
    assert b"/FontFile2" in pdf  # TrueType (Type 42) font program embedded
    assert b"/CreationDate" not in pdf and b"Matplotlib" not in pdf
    svg = paths[1].read_text()
    assert "<text" not in svg and "<dc:date>" not in svg
    assert b"Software" not in paths[2].read_bytes()[:400]
    editable = qq.plot.save(figure, tmp_path / "editable.svg", svg_text="text")
    assert "<text" in editable[0].read_text()
    composed = qq.plot.save(qq.plot.goodness_of_fit(results["population"]),
                            tmp_path / "gof.png", journal="nature", dpi=300)
    assert composed[0].stat().st_size > 0
    from PIL import Image

    with Image.open(composed[0]) as image:
        assert abs(image.size[0] / 300 * 25.4 - 183) < 1  # Nature double column
    with Image.open(paths[2]) as image:
        assert abs(image.size[0] / 600 * 25.4 - 86) < 1  # CPT:PSP single column
    for kwargs, message in [
        ({"path": tmp_path / "x.jpg"}, "suffix"),
        ({"path": tmp_path / "x", "width": 500}, "width"),
        ({"path": tmp_path / "x", "height": 400}, "height"),
        ({"path": tmp_path / "x", "dpi": 72}, "dpi"),
        ({"path": tmp_path / "x", "journal": "other"}, "journal"),
        ({"path": tmp_path / "x.pdf", "formats": ["svg"]}, "either"),
        ({"path": tmp_path / "x", "svg_text": "glyphs"}, "svg_text"),
    ]:
        with pytest.raises(qq.plot.StyleError, match=message):
            qq.plot.save(figure, **kwargs)
    with pytest.raises(qq.plot.StyleError, match="plotnine figure"):
        qq.plot.save(object(), tmp_path / "x")


# Saved tables, model overlays, target attainment and the HTML report ---------

def _write_table(directory, rows, columns, *, producer="mcp", name="table.csv"):
    """Write a CSV and a table manifest the way the MCP save_csv tool does."""
    import csv
    import hashlib
    import io

    text = io.StringIO(newline="")
    writer = csv.writer(text, lineterminator="\n")
    writer.writerow([c["name"] for c in columns])
    writer.writerows(rows)
    raw = text.getvalue().encode()
    counts = {"pre": sum(r[1] == "pre" for r in rows), "post": sum(r[1] == "post" for r in rows)}
    pairs = sum(1 for a, b in zip(rows, rows[1:]) if a[0] == b[0])
    manifest = {"schema": "qspquest.table-manifest/v1", "producer": producer,
                "table": {"filename": name, "sha256": hashlib.sha256(raw).hexdigest(),
                          "bytes": len(raw), "rows": len(rows), "columns": columns},
                "events": {"rule": "nondecreasing_time;equal_time_requires_pre_then_post",
                           "pre": counts["pre"], "post": counts["post"], "event_pairs": pairs},
                "model": {"id": "synthetic_cells", "version": "1", "archive_sha256": None,
                          "provenance": None},
                "engine_identity": None, "inputs": {}, "qualification": {}}
    path = Path(directory) / name
    path.write_bytes(raw)
    Path(str(path) + ".manifest.json").write_text(json.dumps(manifest))
    return path


CELL_COLUMNS = [
    {"name": "time_d", "unit": "d", "kind": "time", "native_name": None, "meaning": "Time"},
    {"name": "side", "unit": None, "kind": "event_side", "native_name": None, "meaning": "Side"},
    {"name": "conc_mg_l", "unit": "mg/L", "kind": "output", "native_name": "conc", "meaning": "Concentration"},
    {"name": "cells", "unit": "cell/mm^3", "kind": "output", "native_name": "cells", "meaning": "Cells"},
    {"name": "ratio", "unit": None, "kind": "output", "native_name": None, "meaning": "Unitless readout"},
]
CELL_ROWS = [(0, "pre", 0.0, 100.0, 1.0), (0, "post", 5.0, 100.0, 1.0),
             (7, "post", 2.5, 40.0, 0.4), (14, "post", 1.2, 12.0, 0.2)]


def test_read_table_takes_units_from_the_manifest(tmp_path):
    from qspquest.first_run import synthetic_inputs
    from qspquest.result_bundle import write_bundle

    source, request = synthetic_inputs()
    write_bundle(tmp_path / "first-run", qq.CompiledModel(source).run(request), source, request,
                 producer="python")
    first = qq.read_table(tmp_path / "first-run" / "first-run.csv")
    assert first["model_id"] == "synthetic_one_compartment"
    assert [(o["name"], o["column"], o["unit"]) for o in first["outputs"]] == [("cp", "cp_mg_per_L", "mg/L")]
    assert _labels(_built(qq.plot.profile(first))) == ("Time (h)", "cp (mg/L)")
    table = qq.read_table(_write_table(tmp_path, CELL_ROWS, CELL_COLUMNS))
    assert table["sides"][:2] == ["pre", "post"] and table["time_unit"] == "d"
    assert _built(qq.plot.profile(table, output="cells")).labels.y == "cells (cell/mm^3)"
    assert qq.plot.profile(table, output="ratio").labels.y == "ratio (unit not declared)"
    frame = qq.to_frame(table)
    assert set(frame["output"]) == {"conc", "cells", "ratio"} and list(frame["time_unit"].unique()) == ["d"]


def test_read_table_rejects_altered_or_inconsistent_files(tmp_path):
    def check(rows, message, columns=CELL_COLUMNS, edit=None):
        directory = tmp_path / str(len(list(tmp_path.iterdir())))
        directory.mkdir()
        path = _write_table(directory, rows, columns)
        if edit:
            edit(path)
        with pytest.raises(QSPQuestError, match=message):
            qq.read_table(path)

    def append(path):
        path.write_bytes(path.read_bytes() + b"21,post,0.1,5,0.1\n")

    check(CELL_ROWS, "SHA-256", edit=append)
    check([(0, "post", 1.0, 1.0, 1.0), (0, "pre", 1.0, 1.0, 1.0)], "pre then post")
    check([(7, "post", 1.0, 1.0, 1.0), (0, "post", 1.0, 1.0, 1.0)], "nondecreasing")
    check(CELL_ROWS, "time unit", columns=[dict(CELL_COLUMNS[0], unit=None)] + CELL_COLUMNS[1:])
    def rename(path):
        manifest = Path(str(path) + ".manifest.json")
        data = json.loads(manifest.read_text())
        data["table"]["filename"] = "other.csv"
        manifest.write_text(json.dumps(data))

    check(CELL_ROWS, "different CSV", edit=rename)
    with pytest.raises(QSPQuestError, match="ordinary file"):
        qq.read_table(tmp_path / "missing.csv")


def test_labelled_collections_overlay_models(results, tmp_path):
    table = qq.read_table(_write_table(tmp_path, CELL_ROWS, CELL_COLUMNS))
    other = copy.deepcopy(table)
    for column in other["outputs"]:
        column["values"] = [v * 1.5 for v in column["values"]]
    pairs = {("Model A", "Week 2"): table, ("Model B", "Week 2"): other, ("Model A", "Week 3"): table}
    frame = qq.to_frame(pairs)
    assert list(frame["group"].cat.categories) == ["Model A", "Model B"]
    assert list(frame["panel"].cat.categories) == ["Week 2", "Week 3"]
    figure = _built(qq.plot.profile(pairs, output="cells", log=True))
    assert type(figure.facet).__name__ == "facet_wrap"
    with pytest.raises(QSPQuestError, match="every entry or for none"):
        qq.to_frame({"Model A": table, ("Model B", "Week 2"): other})
    too_many = {(f"Model {i}", "Week 2"): table for i in range(4)}
    with pytest.raises(QSPQuestError, match="at most 3 series"):
        qq.plot.profile(too_many, output="cells")
    comparison = results["comparison"]
    scaled = copy.deepcopy(comparison)
    for record in scaled["regimens"].values():
        record["curve_percentiles"] = [[1.2 * v for v in row] for row in record["curve_percentiles"]]
    models = {"Model A": comparison, "Model B": scaled}
    bands = _built(qq.plot.regimens(models, target=2.0))
    assert _labels(bands) == ("Time (h)", "cp (mg/L)")
    assert set(bands.data["model"]) == {"Model A", "Model B"}
    assert "model" in qq.to_frame(models, "subjects").columns
    renamed = copy.deepcopy(scaled)
    for record in renamed["regimens"].values():
        for metrics in record["subject_metrics"]:
            metrics["output"] = "other"
    with pytest.raises(QSPQuestError, match="different outputs"):
        qq.plot.regimens({"Model A": comparison, "Model B": renamed})


@pytest.mark.parametrize("percentiles", [None, [0.05, 0.95]])
def test_regimen_figures_accept_one_or_two_recorded_percentiles(percentiles):
    from qspquest._analysis import native

    model = qq.CompiledModel((ROOT / "examples/synthetic/absorption.qsp").read_text())
    run = qq.run_request(end=24, times=[0, 1, 2, 4, 8, 24],
                         administrations=[qq.bolus("depot", 180, "mg")],
                         solver="analytic_linear_pk")
    arguments = {"regimens": [{"label": "once", "run": run}], "output": "cp"}
    if percentiles is not None:
        arguments["percentiles"] = percentiles
    comparison = native(model, "compare_regimens", arguments)
    record = comparison["regimens"]["once"]
    assert record["percentiles"] == (percentiles or [0.5])
    figure = _built(qq.plot.regimens(comparison))
    assert list(figure.layers[0].data["ymin"]) == pytest.approx(record["curve_percentiles"][0])
    assert list(figure.layers[0].data["ymax"]) == pytest.approx(record["curve_percentiles"][-1])


def test_blq_figure_keeps_native_vpc_strata_separate():
    model = qq.CompiledModel((ROOT / "examples/synthetic/absorption.qsp").read_text())
    subjects = _subjects(model, count=2)
    for index, subject in enumerate(subjects):
        subject["covariates"] = {"arm": "below" if index == 0 else "above"}
        for observation in subject["observations"]:
            observation["lloq"] = qq.quantity(2, "mg/L")
            observation["value"] = qq.quantity(2 if index == 0 else 3, "mg/L")
            if index == 0:
                observation["censoring"] = "left"
    result = model.visual_predictive_check(
        subjects, {"cl": qq.quantity(2, "L/h")}, [[0.01]], ERROR,
        random_effects=["cl"], stratify=["arm"], replicates=5, bins=[0, 2, 24])
    figure = _built(qq.plot.vpc_blq(result))
    line = figure.layers[1].data
    assert len(figure.layout.layout) == 2
    assert sorted(line.groupby("PANEL", observed=True)["y"].unique().map(list)) == [[0.0], [1.0]]


def test_target_attainment_counts_subjects_with_wilson_intervals(results):
    from qspquest.tables import target_attainment, wilson_interval

    assert wilson_interval(0, 10) == pytest.approx((0.0, 0.27753), abs=1e-5)
    assert wilson_interval(5, 10) == pytest.approx((0.23659, 0.76341), abs=1e-5)
    comparison = results["comparison"]
    subjects = qq.to_frame(comparison, "subjects")
    target = float(subjects[subjects["metric"] == "cmax"]["value"].median())
    table = target_attainment(comparison, "cmax", target, direction="above")
    for row in table.itertuples():
        values = subjects[(subjects["metric"] == "cmax") & (subjects["regimen"] == row.regimen)]["value"]
        assert row.met == int((values >= target).sum()) and row.subjects == len(values)
        assert row.lower <= row.attainment <= row.upper
    figure = _built(qq.plot.target_attainment(comparison, "cmax", target, direction="above"))
    assert figure.labels.x == f"Subjects with Cmax of cp ≥ {target:g} mg/L"
    models = {"Model A": comparison, "Model B": comparison}
    assert set(target_attainment(models, "cmin", 1.0)["model"]) == {"Model A", "Model B"}
    _built(qq.plot.target_attainment(models, "cmin", 1.0))
    with pytest.raises(QSPQuestError, match="direction"):
        target_attainment(comparison, "cmax", 1.0, direction="within")
    with pytest.raises(QSPQuestError, match="finite"):
        target_attainment(comparison, "cmax", float("nan"))
    with pytest.raises(QSPQuestError, match="metric must be"):
        target_attainment(comparison, "half_life", 1.0)
    with pytest.raises(QSPQuestError, match="compare_regimens"):
        target_attainment(results["vpc"], "cmax", 1.0)


def test_report_uses_the_figure_palette_and_overlays_models(results, tmp_path):
    palette = qq.plot.palette()
    comparison = results["comparison"]
    scaled = copy.deepcopy(comparison)
    path = qq.write_analysis_report(tmp_path / "report.html", comparison={"A": comparison, "B": scaled},
                                    runs={"Run": results["run"]})
    html = Path(path).read_text()
    for colour in palette[:2]:
        assert f'stroke="{colour}"' in html
    assert "#2563eb" not in html and "A: 180 mg once" in html and "<h2>Trajectories</h2>" in html
    assert 'stroke-dasharray="6 3"' in html  # second regimen of each model
    with pytest.raises(QSPQuestError, match="compare_regimens"):
        qq.write_analysis_report(tmp_path / "bad.html", comparison=results["vpc"])
    other = copy.deepcopy(results["run"])
    other["outputs"][0]["unit"] = "ug/L"
    with pytest.raises(QSPQuestError, match="different units"):
        qq.write_analysis_report(tmp_path / "bad.html", runs={"a": results["run"], "b": other})
