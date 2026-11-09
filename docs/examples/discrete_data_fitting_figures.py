"""Figures for the discrete-data fitting tutorial.

Fits gamma-poisson-fit.json and probit-normal-binomial-fit.json through the
Python binding, checks the estimates and likelihoods against the documented
values and closed forms, and writes two PNG figures to docs/tutorials/figures/.
Requires the ``docs`` extra (numpy, scipy, pandas and plotnine).
"""

import json
import math
from pathlib import Path

import numpy as np
import pandas as pd
from numpy.polynomial.hermite_e import hermegauss
from plotnine import (
    aes, annotate, element_text, facet_wrap, geom_crossbar, geom_hline, geom_line, geom_point,
    geom_ribbon, geom_step, ggplot, labs, scale_color_manual, scale_linetype_manual,
    scale_x_continuous, scale_y_continuous, theme,
)
from plotnine.composition import Beside
from scipy.special import gammaln
from scipy.stats import binom, gamma, nbinom, norm, poisson

import qspquest as qq
from figure_style import CATEGORICAL, FILL_LIGHT, INK, MUTED, save, style

HERE = Path(__file__).resolve().parent
COUNTS = json.loads((HERE / "gamma-poisson-fit.json").read_text())
LITTERS = json.loads((HERE / "probit-normal-binomial-fit.json").read_text())


def gamma_poisson():
    """Fit the count example and check it against the negative binomial closed form."""
    result = qq.fit_gamma_poisson(COUNTS)
    assert result["status"] == "converged" and result["active_bounds"] == []
    shape = result["parameters"]["shape"]["value"]
    rate = result["parameters"]["rate"]["value"]
    assert result["parameters"]["rate"]["unit"] == "h"
    assert abs(shape - 0.772885) < 5e-7 and abs(rate - 0.128814) < 5e-7
    assert abs(result["negative_log_likelihood"] - 14.29792892) < 5e-9
    counts = np.array([o["count"] for o in COUNTS["observations"]])
    exposure = np.array([o["exposure"]["value"] for o in COUNTS["observations"]])
    assert all(o["exposure"]["unit"] == "h" for o in COUNTS["observations"])
    # Marginal negative binomial: P(y) = G(y+a)/(G(a) y!) p^a (1-p)^y, p = b/(b+e).
    p = rate / (rate + exposure)
    log_pmf = (gammaln(counts + shape) - gammaln(shape) - gammaln(counts + 1)
               + shape * np.log(p) + counts * np.log1p(-p))
    assert abs(-log_pmf.sum() - result["negative_log_likelihood"]) < 1e-9
    assert np.allclose(log_pmf, nbinom.logpmf(counts, shape, p), atol=1e-12)
    # With equal exposures, the fitted marginal mean equals the mean count per hour.
    assert abs(shape / rate - counts.sum() / exposure.sum()) < 1e-6
    return shape, rate, counts, exposure


def counts_figure():
    """Figure 1: count distribution and latent event rates."""
    shape, rate, counts, exposure = gamma_poisson()
    assert len(set(exposure)) == 1
    p = rate / (rate + exposure[0])
    mean = shape / rate * exposure[0]
    support = np.arange(0, 41)
    cdf = pd.concat([
        pd.DataFrame({"count": support, "cdf": nbinom.cdf(support, shape, p),
                      "model": "Gamma–Poisson (fitted)"}),
        pd.DataFrame({"count": support, "cdf": poisson.cdf(support, mean),
                      "model": f"Poisson, same mean ({mean:.0f})"}),
    ])
    models = list(dict.fromkeys(cdf["model"]))
    cdf["model"] = pd.Categorical(cdf["model"], models)
    ordered = np.sort(counts)
    empirical = pd.DataFrame({"count": ordered, "cdf": np.arange(1, len(ordered) + 1) / len(ordered)})
    steps = pd.DataFrame({"count": np.concatenate([[0], ordered, [40]]),
                          "cdf": np.concatenate([[0], empirical["cdf"], [1]])})
    distribution = (ggplot()
                    + geom_step(steps, aes("count", "cdf"), color=MUTED, size=0.4)
                    + geom_step(cdf, aes("count", "cdf", color="model", linetype="model"), size=0.8)
                    + geom_point(empirical, aes("count", "cdf"), color=INK, fill="white",
                                 shape="o", size=2.4, stroke=0.9)
                    + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[1]])
                    + scale_linetype_manual(values=["solid", "dashed"])
                    + scale_x_continuous(breaks=[0, 5, 10, 15, 20, 25, 30, 35, 40])
                    + scale_y_continuous(limits=(0, 1))
                    + labs(x="Events per 1 h exposure", y="Cumulative probability",
                           title="A  Distribution of counts")
                    + style(width=4.6, height=4.2)
                    + theme(legend_position=(0.97, 0.05), legend_justification=(1, 0),
                            legend_direction="vertical"))
    # Conditional (posterior) distribution of each latent rate: Gamma(a + y, b + e).
    order = np.argsort(counts, kind="stable")
    rows = pd.DataFrame({
        "row": [f"{i + 1}" for i in order],
        "observed": counts[order] / exposure[order],
        "mean": (shape + counts[order]) / (rate + exposure[order]),
        "low": gamma.ppf(0.05, shape + counts[order], scale=1 / (rate + exposure[order])),
        "high": gamma.ppf(0.95, shape + counts[order], scale=1 / (rate + exposure[order])),
    })
    rows["row"] = pd.Categorical(rows["row"], list(rows["row"]))
    assert all(rows["low"] < rows["mean"]) and all(rows["mean"] < rows["high"])
    # Shrinkage: each conditional mean lies between the row's own rate and the shared mean.
    shared = shape / rate
    assert all((rows["mean"] - rows["observed"]) * (shared - rows["observed"]) >= 0)
    assert all(abs(rows["mean"] - shared) <= abs(rows["observed"] - shared) + 1e-12)
    rates = (ggplot(rows, aes("row"))
             + geom_hline(yintercept=shared, color=MUTED, linetype="dotted", size=0.5)
             + geom_crossbar(aes(y="mean", ymin="low", ymax="high"), color=CATEGORICAL[0],
                             fill="white", width=0.45, size=0.6)
             + geom_point(aes(y="observed"), color=INK, fill="white", shape="o", size=2.4,
                          stroke=0.9)
             + annotate("text", x=0.55, y=shared + 0.6, label=f"α/β = {shared:.1f} /h",
                        ha="left", va="bottom", color=INK, size=8)
             + scale_y_continuous(breaks=[0, 5, 10, 15, 20, 25])
             + labs(x="Row (ordered by count)", y="Event rate (1/h)",
                    title="B  Latent event rate by row")
             + style(width=4.0, height=4.2))
    plot = Beside([distribution, rates]) + theme(figure_size=(8.6, 4.4))
    return save(plot, "discrete-data-fitting-counts")


def outcome_probabilities(mean, sd, total, nodes=80):
    """P(Y = k) for Y | Z ~ Binomial(total, Phi(mean + sd*Z)), Z ~ N(0, 1), by Gauss-Hermite."""
    z, w = hermegauss(nodes)
    w = w / w.sum()
    p = norm.cdf(mean + sd * z)
    return np.array([np.dot(w, binom.pmf(k, total, p)) for k in range(total + 1)])


def litters_figure():
    """Figure 2: observed and expected outcome frequencies per group."""
    result = qq.fit_probit_normal_binomial(LITTERS)
    assert result["status"] == "converged" and result["active_bounds"] == []
    assert all(abs(m) < 1e-6 for m in result["means"])
    assert all(abs(s - 1) < 1e-6 for s in result["sds"])
    assert abs(result["negative_log_likelihood"] - 6.59167373) < 5e-9
    litters = pd.DataFrame(LITTERS["observations"])
    nll, binomial_nll, frames = 0.0, 0.0, []
    for group in (0, 1):
        subset = litters[litters["group"] == group]
        (total,) = set(subset["total"])
        expected = outcome_probabilities(result["means"][group], result["sds"][group], total)
        # For mean 0 and SD 1, Phi(Z) is uniform on (0, 1), so P(Y = k) = 1/(total + 1).
        assert np.allclose(expected, 1 / (total + 1), atol=1e-9)
        nll -= sum(math.log(expected[k]) for k in subset["successes"])
        pooled = subset["successes"].sum() / subset["total"].sum()
        binomial = binom.pmf(np.arange(total + 1), total, pooled)
        binomial_nll -= sum(math.log(binomial[k]) for k in subset["successes"])
        n = len(subset)
        observed = np.bincount(subset["successes"], minlength=total + 1) / n
        label = f"Group {group} ({n} litters of {total})"
        frames.append(pd.DataFrame({
            "k": np.arange(total + 1), "observed": observed, "expected": expected,
            "binomial": binomial, "low": binom.ppf(0.05, n, expected) / n,
            "high": binom.ppf(0.95, n, expected) / n, "group": label}))
    assert abs(nll - result["negative_log_likelihood"]) < 1e-8
    assert abs(nll - 6 * math.log(3)) < 1e-8
    assert abs(binomial_nll - 10 * math.log(2)) < 1e-12  # pooled p = 1/2 in each group
    data = pd.concat(frames)
    lines = pd.concat([
        data[["k", "group"]].assign(value=data["expected"],
                                    model=f"Probit–normal, fitted (NLL {nll:.3f})"),
        data[["k", "group"]].assign(value=data["binomial"],
                                    model=f"Binomial, p = 0.5 (NLL {binomial_nll:.3f})"),
    ])
    lines["model"] = pd.Categorical(lines["model"], list(dict.fromkeys(lines["model"])))
    plot = (ggplot()
            + geom_ribbon(data, aes("k", ymin="low", ymax="high"), fill=FILL_LIGHT, alpha=0.3)
            + geom_line(lines, aes("k", "value", color="model", linetype="model"), size=0.8)
            + geom_point(data, aes("k", "observed"), color=INK, fill="white", shape="o", size=2.6,
                         stroke=0.9)
            + facet_wrap("~group", ncol=2)
            + scale_color_manual(values=[CATEGORICAL[0], CATEGORICAL[1]])
            + scale_linetype_manual(values=["solid", "dashed"])
            + scale_x_continuous(breaks=[0, 1, 2])
            + scale_y_continuous(limits=(0, 1), breaks=[0, 1 / 3, 2 / 3, 1],
                                 labels=["0", "1/3", "2/3", "1"])
            + labs(x="Successes per litter", y="Proportion of litters")
            + style(width=7.0, height=3.8)
            + theme(strip_text=element_text(color=INK, weight="bold", ha="left", size=10)))
    return save(plot, "discrete-data-fitting-litters")


def main():
    for figure in (counts_figure, litters_figure):
        print(figure().name)


if __name__ == "__main__":
    main()
