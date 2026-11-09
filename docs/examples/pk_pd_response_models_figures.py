"""Figures for the direct, effect-compartment and indirect response tutorial.

Repeats the simulations of pk_pd_response_models.py, asserts every plotted
simulated value against the closed-form or independent scipy solution, and
writes six PNG figures to docs/tutorials/figures/. Requires the ``docs`` extra.
"""

import math

import numpy as np
import pandas as pd
from plotnine import (
    aes, annotate, arrow, element_text, facet_wrap, geom_hline, geom_line, geom_path,
    geom_point, geom_raster, geom_rect, geom_segment, geom_text, geom_vline, ggplot, labs,
    scale_color_manual, scale_fill_gradientn, scale_fill_manual, scale_linetype_manual, scale_size_manual,
    scale_x_continuous, scale_x_log10, scale_y_continuous, theme,
)
from plotnine.composition import Beside

import qspquest as qq
from figure_style import CATEGORICAL, FILL_LIGHT, INK, MUTED, SEQUENTIAL, save, style
from pk_pd_response_models import (
    BOUNDS, CL, EC50, EMAX, F, HILL, K, KA, R0, ce_analytic, cp_analytic, emax, emax_fit,
    idr_reference, model, oral_doses, schedule, simulate,
)

TMAX = math.log(KA / K) / (KA - K)
DOSE_COLORS = dict(zip(("150 mg", "300 mg", "600 mg"), CATEGORICAL))
REGIMENS = (("150 mg q12h", 150, 12), ("300 mg q24h", 300, 24), ("600 mg q48h", 600, 48))
RESPONSES = {"r_kin": "R1, inhibition of kin", "r_kout": "R2, stimulation of kout"}


def check(output, engine, reference):
    """Assert the comparison bound of pk_pd_response_models.py at every point."""
    absolute, relative = BOUNDS[output]
    engine, reference = np.asarray(engine, dtype=float), np.asarray(reference, dtype=float)
    bound = absolute + relative * np.maximum(np.abs(engine), np.abs(reference))
    assert np.all(np.abs(engine - reference) <= bound), output


def grid(end, step):
    return np.round(np.arange(0, end + step / 2, step), 10)


def emax_curves():
    """Figure 1: concentration-effect relationship for three Hill coefficients."""
    conc = np.geomspace(0.05, 50, 400)
    curves = []
    for gamma in (1.0, 1.5, 3.0):
        x = (conc / EC50) ** gamma
        label = f"γ = {gamma:g}" + (" (model)" if gamma == HILL else "")
        curves.append(pd.DataFrame({"conc": conc, "effect": EMAX * x / (1 + x), "gamma": label}))
    curves = pd.concat(curves)
    order = list(dict.fromkeys(curves["gamma"]))
    curves["gamma"] = pd.Categorical(curves["gamma"], order)
    # Simulated points: one steady-state interval of 600 mg q48h, on both limbs.
    offsets = [0.25, 0.5, 1, 1.5, 2, 3, 4, 6, 8, 12, 16, 24, 32, 40, 48]
    times = np.array([288.0 + h for h in offsets])
    doses = schedule(600, 48, 7)
    sim = simulate(model("direct-emax.qsp"), times, oral_doses(600, 48, 7), end=336)
    check("cp", sim["cp"], cp_analytic(times, doses))
    check("effect", sim["effect"], emax(cp_analytic(times, doses)))
    rising = cp_analytic(times + 1e-4, doses) > cp_analytic(times, doses)
    points = pd.DataFrame({"conc": sim["cp"], "effect": sim["effect"],
                           "limb": np.where(rising, "Rising limb", "Falling limb")})
    points["limb"] = pd.Categorical(points["limb"], ["Rising limb", "Falling limb"])
    points = points.sort_values("limb", ascending=False)  # draw rising-limb points last
    window = cp_analytic(np.round(np.arange(288, 336.0001, 0.05), 10), doses)
    low, high = float(window.min()), float(window.max())
    assert abs(low - 1.26) < 0.01 and abs(high - 7.15) < 0.01, (low, high)
    plot = (ggplot()
            + geom_rect(aes(xmin=low, xmax=high, ymin=-np.inf, ymax=np.inf),
                        data=pd.DataFrame({"x": [0]}), fill=FILL_LIGHT, alpha=0.18)
            + geom_hline(yintercept=EMAX / 2, color=MUTED, linetype="dotted", size=0.5)
            + geom_vline(xintercept=EC50, color=MUTED, linetype="dotted", size=0.5)
            + geom_hline(yintercept=EMAX, color=MUTED, linetype="dashed", size=0.5)
            + geom_line(curves, aes("conc", "effect", color="gamma"), size=0.85)
            + geom_point(points, aes("conc", "effect", fill="limb", size="limb"), color=INK,
                         shape="o", stroke=0.8)
            + annotate("text", x=0.06, y=EMAX + 1.5, label="Emax = 80 %", ha="left", va="bottom",
                       color=INK, size=8.5)
            + annotate("text", x=EC50 * 1.08, y=3, label="EC50 = 2.5 mg/L", ha="left",
                       color=INK, size=8.5)
            + annotate("text", x=math.sqrt(low * high), y=92,
                       label="Cp range, 600 mg q48h at steady state", ha="center",
                       color=INK, size=8.5)
            + scale_color_manual(values=CATEGORICAL)
            + scale_fill_manual(values=[INK, "white"])
            + scale_size_manual(values=[1.6, 3.0])
            + scale_x_log10(breaks=[0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 25, 50],
                            labels=["0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "10", "25", "50"])
            + scale_y_continuous(limits=(0, 96), breaks=[0, 20, 40, 60, 80])
            + labs(x="Plasma concentration Cp (mg/L, log scale)", y="Effect E (%)")
            + style(width=7.0, height=4.4))
    return save(plot, "pk-pd-emax-curves")


def hysteresis():
    """Figure 2: plasma and effect-site concentrations and the effect-Cp loop."""
    compiled = model("effect-compartment.qsp")
    doses = schedule(300, 24, 1)
    dense = np.linspace(0, 96, 1921)
    cp_line, ce_line = cp_analytic(dense, doses), ce_analytic(dense, doses)
    sample = grid(48, 2.0)
    sim = simulate(compiled, sample, oral_doses(300, 24, 1), end=96)
    check("cp", sim["cp"], cp_analytic(sample, doses))
    check("ce", sim["ce"], ce_analytic(sample, doses))
    check("effect", sim["effect"], emax(ce_analytic(sample, doses)))
    fine = grid(96, 0.05)
    tce = float(fine[np.argmax(ce_analytic(fine, doses))])
    assert abs(tce - 13.25) < 1e-9, tce

    window = dense <= 48
    lines = pd.concat([
        pd.DataFrame({"time": dense[window], "conc": cp_line[window], "kind": "Cp, plasma"}),
        pd.DataFrame({"time": dense[window], "conc": ce_line[window], "kind": "Ce, effect site"}),
    ])
    points = pd.concat([
        pd.DataFrame({"time": sample, "conc": sim["cp"], "kind": "Cp, plasma"}),
        pd.DataFrame({"time": sample, "conc": sim["ce"], "kind": "Ce, effect site"}),
    ])
    for frame in (lines, points):
        frame["kind"] = pd.Categorical(frame["kind"], ["Cp, plasma", "Ce, effect site"])
    peaks = pd.DataFrame({"time": [TMAX, tce], "conc": [cp_analytic([TMAX], doses)[0],
                                                       ce_analytic([tce], doses)[0]],
                          "label": [f"Cp peak, {TMAX:.2f} h", f"Ce peak, {tce:.2f} h"]})
    left = (ggplot()
            + geom_line(lines, aes("time", "conc", color="kind"), size=0.85)
            + geom_point(points, aes("time", "conc", color="kind"), fill="white", shape="o",
                         size=2.0, stroke=0.8)
            + geom_text(peaks, aes("time", "conc", label="label"), nudge_x=1.2, nudge_y=0.17,
                        ha="left", color=INK, size=8.5)
            + scale_color_manual(values=CATEGORICAL[:2])
            + scale_x_continuous(breaks=[0, 12, 24, 36, 48])
            + scale_y_continuous(limits=(0, 3.4))
            + labs(x="Time after dose (h)", y="Concentration (mg/L)", title="A  Concentrations")
            + style(width=3.6, height=4.4))

    # Loop of effect against Cp from 0 to 96 h, with the direct-model curve for reference.
    loop = pd.DataFrame({"cp": cp_line, "effect": emax(ce_line)})
    direct = pd.DataFrame({"cp": np.linspace(0, 3.1, 200)})
    direct["effect"] = emax(direct["cp"])
    labelled = [1, 2, 4, 8, 13.25, 24, 36, 48]
    marks = simulate(compiled, labelled, oral_doses(300, 24, 1), end=96)
    check("cp", marks["cp"], cp_analytic(labelled, doses))
    check("effect", marks["effect"], emax(ce_analytic(labelled, doses)))
    nudges = {1: (0.13, 2.4), 2: (-0.12, 2.4), 4: (-0.17, 0.0), 8: (0.18, 1.2),
              13.25: (0.3, 2.0), 24: (-0.2, 1.6), 36: (-0.2, 1.6), 48: (-0.2, 1.6)}
    labels = pd.DataFrame({"cp": marks["cp"], "effect": marks["effect"],
                           "label": [f"{t:g} h" for t in labelled]})
    labels["x"] = labels["cp"] + [nudges[t][0] for t in labelled]
    labels["y"] = labels["effect"] + [nudges[t][1] for t in labelled]
    arrows = []
    for t0 in (0.55, 6.0, 18.0, 30.0):
        i = int(np.searchsorted(dense, t0))
        arrows.append({"x": cp_line[i], "y": emax(ce_line[i]),
                       "xend": cp_line[i + 2], "yend": emax(ce_line[i + 2])})
    arrows = pd.DataFrame(arrows)
    level = 2.0
    pair = pd.DataFrame({"cp": [level, level], "effect": [1.3, 35.3]})
    rise_fall = effect_at_matched_cp(compiled, doses, level)
    assert np.allclose(np.round(rise_fall, 1), pair["effect"]), rise_fall
    right = (ggplot()
             + geom_line(direct, aes("cp", "effect"), color=MUTED, linetype="dashed", size=0.6)
             + geom_path(loop, aes("cp", "effect"), color=CATEGORICAL[1], size=0.85)
             + geom_segment(arrows, aes(x="x", y="y", xend="xend", yend="yend"),
                            color=CATEGORICAL[1], size=0.85,
                            arrow=arrow(length=0.09, type="closed"))
             + geom_vline(xintercept=level, color=MUTED, linetype="dotted", size=0.5)
             + geom_point(labels, aes("cp", "effect"), color=CATEGORICAL[1], fill="white",
                          shape="o", size=2.2, stroke=0.8)
             + geom_text(labels, aes("x", "y", label="label"), color=INK, size=8)
             + annotate("text", x=3.1, y=42, label="Direct\nmodel", color=MUTED, size=8.5,
                        ha="center", va="top")
             + annotate("text", x=level - 0.04, y=49, label="Cp = 2.0 mg/L", ha="right",
                        color=MUTED, size=8)
             + scale_x_continuous(limits=(0, 3.15), breaks=[0, 0.5, 1, 1.5, 2, 2.5, 3])
             + scale_y_continuous(limits=(0, 50))
             + labs(x="Plasma concentration Cp (mg/L)", y="Effect E (%)",
                    title="B  Effect against Cp, 0–96 h")
             + style(width=4.4, height=4.4))
    plot = Beside([left, right]) + theme(figure_size=(8.2, 4.4))
    return save(plot, "pk-pd-hysteresis")


def effect_at_matched_cp(compiled, doses, level):
    from scipy.optimize import brentq
    def gap(t):
        return float(cp_analytic([t], doses)[0]) - level
    times = [brentq(gap, 0.01, TMAX, xtol=1e-12), brentq(gap, TMAX, 96, xtol=1e-12)]
    sim = simulate(compiled, times, oral_doses(300, 24, 1), end=96)
    check("effect", sim["effect"], emax(ce_analytic(times, doses)))
    return sim["effect"]


def indirect_time_courses():
    """Figure 3: R1 and R2 after single doses of 150, 300 and 600 mg."""
    compiled = model("indirect-response.qsp")
    dense, sample = grid(120, 0.05), grid(120, 4.0)
    expected_nadirs = {"r_kin": (22.90, 25.55, 29.00), "r_kout": (17.50, 17.55, 18.00)}
    lines, points, nadirs = [], [], []
    for j, (label, amount) in enumerate(zip(DOSE_COLORS, (150, 300, 600))):
        doses = schedule(amount, 24, 1)
        ref = idr_reference(dense, doses, 120)
        sim = simulate(compiled, dense, oral_doses(amount, 24, 1))
        for name, panel in RESPONSES.items():
            check(name, sim[name], ref[name])
            i = int(np.argmin(sim[name]))
            assert abs(dense[i] - expected_nadirs[name][j]) < 1e-9, (name, amount, dense[i])
            lines.append(pd.DataFrame({"time": dense, "value": ref[name], "dose": label,
                                       "panel": panel}))
            on_sample = np.isin(dense, sample)
            points.append(pd.DataFrame({"time": dense[on_sample], "value": sim[name][on_sample],
                                        "dose": label, "panel": panel}))
            nadirs.append({"time": dense[i], "value": sim[name][i], "dose": label,
                           "panel": panel, "label": f"{label}: nadir at {dense[i]:.2f} h",
                           "y": 50 - 5.5 * j})
    lines, points, nadirs = pd.concat(lines), pd.concat(points), pd.DataFrame(nadirs)
    for frame in (lines, points, nadirs):
        frame["dose"] = pd.Categorical(frame["dose"], list(DOSE_COLORS))
        frame["panel"] = pd.Categorical(frame["panel"], list(RESPONSES.values()))
    plot = (ggplot()
            + geom_hline(yintercept=R0, color=INK, linetype="dashed", size=0.5)
            + geom_line(lines, aes("time", "value", color="dose"), size=0.85)
            + geom_point(points, aes("time", "value", color="dose"), fill="white", shape="o",
                         size=1.9, stroke=0.8)
            + geom_point(nadirs, aes("time", "value", color="dose"), shape="^", size=3.0)
            + geom_text(nadirs, aes(x=118, y="y", label="label", color="dose"), ha="right",
                        size=8.5, show_legend=False)
            + annotate("text", x=118, y=R0 + 2.5, label="Baseline R0 = 120 nmol/L",
                       ha="right", va="bottom", color=INK, size=8.5)
            + facet_wrap("~panel", ncol=2)
            + scale_color_manual(values=list(DOSE_COLORS.values()))
            + scale_x_continuous(breaks=[0, 24, 48, 72, 96, 120])
            + scale_y_continuous(limits=(35, 128), breaks=[40, 60, 80, 100, 120])
            + labs(x="Time after dose (h)", y="Response (nmol/L)")
            + style(width=8.0, height=4.2))
    return save(plot, "pk-pd-indirect-responses")


def indirect_dose_map():
    """Figure 4: R1 and R2 over time and single oral dose."""
    compiled = model("indirect-response.qsp")
    doses_mg = [150 * 2 ** (k / 6) for k in range(-12, 19)]  # 37.5 to 1200 mg, log-spaced
    fine = grid(120, 0.05)
    shown = np.isin(fine, grid(120, 0.5))
    rows, nadirs = [], []
    for amount in doses_mg:
        doses = schedule(amount, 24, 1)
        sim = simulate(compiled, fine, oral_doses(amount, 24, 1))
        ref = idr_reference(fine, doses, 120)
        for name, panel in RESPONSES.items():
            check(name, sim[name], ref[name])
            rows.append(pd.DataFrame({"time": fine[shown], "log_dose": math.log10(amount),
                                      "percent": 100 * sim[name][shown] / R0, "panel": panel}))
            nadirs.append({"time": fine[int(np.argmin(sim[name]))],
                           "log_dose": math.log10(amount), "panel": panel})
    surface, nadirs = pd.concat(rows), pd.DataFrame(nadirs)
    r1 = nadirs[nadirs["panel"] == RESPONSES["r_kin"]]["time"].to_numpy()
    r2 = nadirs[nadirs["panel"] == RESPONSES["r_kout"]]["time"].to_numpy()
    assert np.all(np.diff(r1) > 0) and (r1[0], r1[-1]) == (19.9, 33.15), r1
    assert (r2.min(), r2.max()) == (17.5, 19.0), r2
    for frame in (surface, nadirs):
        frame["panel"] = pd.Categorical(frame["panel"], list(RESPONSES.values()))
    ticks = [37.5, 75, 150, 300, 600, 1200]
    plot = (ggplot(surface, aes("time", "log_dose"))
            + geom_raster(aes(fill="percent"), interpolate=True)
            + geom_vline(xintercept=TMAX, color="white", linetype="dashed", size=0.5)
            + geom_path(nadirs, aes("time", "log_dose"), color="white", size=0.9)
            + geom_point(nadirs, aes("time", "log_dose"), color="white", size=0.9)
            + facet_wrap("~panel", ncol=2)
            + scale_fill_gradientn(colors=SEQUENTIAL[::-1], limits=(25, 100),
                                   breaks=[25, 50, 75, 100], name="% of R0")
            + scale_x_continuous(breaks=[0, 24, 48, 72, 96, 120], expand=(0, 0))
            + scale_y_continuous(breaks=[math.log10(t) for t in ticks],
                                 labels=[f"{t:g}" for t in ticks], expand=(0, 0))
            + labs(x="Time after dose (h)", y="Single oral dose (mg, log scale)")
            + style(width=8.0, height=4.2)
            + theme(legend_position="right", legend_title=element_text(color=INK, size=9)))
    return save(plot, "pk-pd-indirect-dose-map")


def regimen_comparison():
    """Figure 5: four endpoints at steady state for three regimens of 300 mg/day."""
    window = (288.0, 336.0)
    dense, sample = grid(window[1], 0.05), grid(window[1], 3.0)
    show = dense >= window[0]
    on_sample = show & np.isin(dense, sample)
    panels = {"cp": "Cp (mg/L)", "effect": "Direct effect E (%)",
              "r_kin": "R1, inhibition of kin (% of R0)",
              "r_kout": "R2, stimulation of kout (% of R0)"}
    direct, indirect = model("direct-emax.qsp"), model("indirect-response.qsp")
    lines, points = [], []
    for label, amount, interval in REGIMENS:
        count = int(window[1] / interval)
        doses = schedule(amount, interval, count)
        d = simulate(direct, dense, oral_doses(amount, interval, count))
        r = simulate(indirect, dense, oral_doses(amount, interval, count))
        ref = idr_reference(dense, doses, window[1])
        exact = {"cp": cp_analytic(dense, doses), "effect": emax(cp_analytic(dense, doses)),
                 "r_kin": ref["r_kin"], "r_kout": ref["r_kout"]}
        engine = {"cp": d["cp"], "effect": d["effect"], "r_kin": r["r_kin"], "r_kout": r["r_kout"]}
        for name, panel in panels.items():
            check(name, engine[name], exact[name])
            scale = 100 / R0 if name.startswith("r_") else 1.0
            lines.append(pd.DataFrame({"time": dense[show], "value": exact[name][show] * scale,
                                       "regimen": label, "panel": panel}))
            points.append(pd.DataFrame({"time": dense[on_sample],
                                        "value": engine[name][on_sample] * scale,
                                        "regimen": label, "panel": panel}))
    lines, points = pd.concat(lines), pd.concat(points)
    for frame in (lines, points):
        frame["regimen"] = pd.Categorical(frame["regimen"], [r[0] for r in REGIMENS])
        frame["panel"] = pd.Categorical(frame["panel"], list(panels.values()))
    css = F * 300 / (CL * 24)
    assert abs(css - 3.65) < 0.005
    mean = pd.DataFrame({"panel": pd.Categorical([panels["cp"]], list(panels.values())),
                         "value": [css]})
    plot = (ggplot()
            + geom_hline(mean, aes(yintercept="value"), color=INK, linetype="dashed", size=0.5)
            + geom_line(lines, aes("time", "value", color="regimen"), size=0.8)
            + geom_point(points, aes("time", "value", color="regimen"), fill="white", shape="o",
                         size=1.7, stroke=0.7)
            + facet_wrap("~panel", ncol=2, scales="free_y")
            + scale_color_manual(values=CATEGORICAL)
            + scale_x_continuous(breaks=[288, 300, 312, 324, 336])
            + labs(x="Time (h)", y="")
            + style(width=8.0, height=6.0)
            + theme(panel_spacing_x=0.06, panel_spacing_y=0.05))
    return save(plot, "pk-pd-regimen-comparison")


def fit_figure():
    """Figure 6: generated effect observations with the fitted and true curves."""
    fit = emax_fit()
    estimates = fit["estimates"]
    assert fit["status"] == "converged"
    assert round(estimates["emax"], 2) == 78.24 and round(estimates["ec50"], 3) == 2.335, estimates
    data = pd.DataFrame(fit["data"])
    doses = schedule(300, 24, 7)
    dense = grid(168, 0.25)
    keep = (dense <= 24) | (dense >= 144)
    times = dense[keep]
    request = qq.run_request(end=168, times=list(times), administrations=oral_doses(300, 24, 7),
                             rtol=1e-10, atol=1e-12,
                             parameters={"emax": qq.quantity(estimates["emax"], "%"),
                                         "ec50": qq.quantity(estimates["ec50"], "mg/L")})
    result = model("direct-emax.qsp").run(request)
    fitted = np.asarray(next(c["values"] for c in result["outputs"] if c["name"] == "effect"))
    x = (cp_analytic(times, doses) / estimates["ec50"]) ** HILL
    check("effect", fitted, estimates["emax"] * x / (1 + x))
    true = emax(cp_analytic(times, doses))

    def day(t):
        return np.where(np.asarray(t) <= 24, "Day 1 (0–24 h)", "Day 7 (144–168 h)")
    curves = pd.concat([
        pd.DataFrame({"time": times, "effect": fitted, "kind": "Fitted model", "day": day(times)}),
        pd.DataFrame({"time": times, "effect": true, "kind": "True parameters",
                      "day": day(times)}),
    ])
    curves["kind"] = pd.Categorical(curves["kind"], ["Fitted model", "True parameters"])
    data["day"] = day(data["time_h"])
    plot = (ggplot()
            + geom_line(curves, aes("time", "effect", color="kind", linetype="kind"), size=0.8)
            + geom_point(data, aes("time_h", "effect_percent"), color=INK, size=2.3)
            + facet_wrap("~day", ncol=2, scales="free_x")
            + scale_color_manual(values=[CATEGORICAL[0], INK])
            + scale_linetype_manual(values=["solid", "dashed"])
            + scale_x_continuous(breaks=[0, 6, 12, 18, 24, 144, 150, 156, 162, 168])
            + scale_y_continuous(limits=(0, 70))
            + labs(x="Time (h)", y="Effect E (%)", color="", linetype="")
            + style(width=8.0, height=3.8))
    return save(plot, "pk-pd-emax-fit")


def main():
    for figure in (emax_curves, hysteresis, indirect_time_courses, indirect_dose_map,
                   regimen_comparison, fit_figure):
        print(figure().name, flush=True)


if __name__ == "__main__":
    main()
