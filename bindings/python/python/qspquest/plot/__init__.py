"""Publication figures for QSPQuest results.

Two layers make up this package. ``qspquest.plot.style`` holds colour definitions,
the theme, scales and journal-size export; it does not depend on
pharmacometric results, and every colour comes from its ``figure-style.json``.
``qspquest.plot.pkpd`` builds figures from QSPQuest results with that layer.
``theme_qspquest`` and the ``scale_*_qspquest`` functions are the style
layer's ``theme_style`` and ``scale_*`` functions under QSPQuest names. The data
behind every figure are available from ``qspquest.to_frame``.
Requires ``qspquest[plot]`` (pandas and plotnine).
"""

from . import style
from .style import (
    StyleError, color, diverging, journals, mode, palette, save, sequential, set_mode, definitions,
)
from .pkpd import (
    bootstrap, exposure, exposure_ratio, goodness_of_fit, likelihood_profile, morris,
    observed_vs_predicted, parameters, profile, random_effects, regimens, residuals, scan,
    steady_state, target_attainment, trace, vpc, vpc_blq,
)

theme_qspquest = style.theme_style
scale_color_qspquest = style.scale_color_categorical
scale_fill_qspquest = style.scale_fill_categorical
scale_color_qspquest_continuous = style.scale_color_sequential
scale_fill_qspquest_continuous = style.scale_fill_sequential
scale_fill_qspquest_diverging = style.scale_fill_diverging

__all__ = [
    "style", "StyleError", "color", "diverging", "journals", "mode", "palette", "save",
    "sequential", "set_mode", "definitions", "theme_qspquest", "scale_color_qspquest",
    "scale_fill_qspquest", "scale_color_qspquest_continuous", "scale_fill_qspquest_continuous",
    "scale_fill_qspquest_diverging", "bootstrap", "exposure", "exposure_ratio",
    "goodness_of_fit", "likelihood_profile", "morris", "observed_vs_predicted", "parameters",
    "profile", "random_effects", "regimens", "residuals", "scan", "steady_state",
    "target_attainment", "trace", "vpc",
    "vpc_blq",
]
