"""Shared plotnine style for QSPQuest tutorial figures.

Colors and fonts are read from the package style definitions
(``qspquest/plot/figure-style.json``), so the tutorials and ``qspquest.plot``
use the same validated brand palette. Requires the ``docs`` extra (pandas and
plotnine).
"""

import json
import os
from importlib.resources import files
from pathlib import Path

from plotnine import element_blank, element_line, element_rect, element_text, theme, theme_minimal

_STYLE = json.loads(files("qspquest.plot").joinpath("figure-style.json").read_text())
_LIGHT, _DARK = _STYLE["modes"]["light"], _STYLE["modes"]["dark"]

# Categorical order (fixed): teal, burgundy, peach. Validated on each surface.
CATEGORICAL = _LIGHT["categorical"]
CATEGORICAL_DARK = _DARK["categorical"]
INK = _LIGHT["ink"]  # titles, axis titles, reference lines
MUTED = _LIGHT["muted"]
GRID = _LIGHT["grid"]
FILL_LIGHT = _LIGHT["fill"]  # ribbons and shaded areas only
SEQUENTIAL = _LIGHT["sequential"]
FONTS = _STYLE["fonts"]["brand"]


def font():
    """First installed font from FONTS (DM Sans is the brand body font)."""
    from matplotlib import font_manager
    installed = {entry.name for entry in font_manager.fontManager.ttflist}
    return next((name for name in FONTS if name in installed), "DejaVu Sans")


# Output directory; QSPQUEST_FIGURE_DIR redirects it, e.g. for CI checks.
FIGURES = Path(os.environ.get(
    "QSPQUEST_FIGURE_DIR", Path(__file__).resolve().parents[1] / "tutorials" / "figures"))


def style(width=7.0, height=4.2, base_size=10):
    """Publication theme: white surface, recessive grid, legend at the bottom."""
    family = font()
    return theme_minimal(base_size=base_size, base_family=family) + theme(
        figure_size=(width, height),
        text=element_text(family=family, color=INK),
        plot_background=element_rect(fill="white", color="white"),
        panel_background=element_rect(fill="white", color="white"),
        panel_grid_minor=element_blank(),
        panel_grid_major=element_line(color=GRID, size=0.4),
        panel_spacing=0.03,
        axis_title=element_text(color=INK, size=base_size),
        axis_text=element_text(color=MUTED, size=base_size - 1),
        strip_text=element_text(color=INK, weight="bold", ha="left", size=base_size),
        strip_background=element_blank(),
        legend_position="bottom",
        legend_title=element_blank(),
        legend_text=element_text(color=INK, size=base_size - 1),
        plot_title=element_text(color=INK, weight="bold", size=base_size + 1, ha="left"),
    )


def save(plot, name, dpi=200):
    """Write the figure as a PNG at the given resolution and return its path."""
    FIGURES.mkdir(parents=True, exist_ok=True)
    path = FIGURES / f"{name}.png"
    plot.save(path, dpi=dpi, verbose=False, metadata={"Software": None})
    return path
