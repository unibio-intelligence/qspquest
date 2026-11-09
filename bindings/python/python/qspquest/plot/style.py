"""Figure style: colour definitions, theme, scales and journal-size export.

The layer is independent of pharmacometric results. Every colour, line
weight, font list and journal preset is read from ``figure-style.json`` next to this
file; no colour is defined in code. Requires pandas and plotnine.
"""

import json
import logging
import math
from pathlib import Path
import warnings

try:
    from plotnine import (
        element_blank, element_line, element_rect, element_text, ggplot, scale_color_gradientn,
        scale_color_manual, scale_fill_gradientn, scale_fill_manual, theme, theme_minimal,
    )
    from plotnine.composition import Compose
except ImportError as error:  # pragma: no cover - exercised without the extra
    raise ImportError("Install qspquest[plot] to make figures") from error

__all__ = [
    "StyleError", "definitions", "modes", "set_mode", "mode", "color", "palette", "sequential",
    "diverging", "line", "point_size", "fill_alpha", "journals", "type_sizes", "font", "theme_style",
    "scale_color_categorical", "scale_fill_categorical", "scale_color_sequential",
    "scale_fill_sequential", "scale_fill_diverging", "size", "save",
]

_STYLE = json.loads((Path(__file__).with_name("figure-style.json")).read_text())
_MODE = ["light"]
_MM = 25.4
_PT = math.sqrt(math.pi)  # plotnine multiplies line and point sizes by this factor


class StyleError(ValueError):
    """A style request that the style definitions or journal presets cannot satisfy."""


def definitions():
    """Return a copy of all style definitions."""
    return json.loads(json.dumps(_STYLE))


def modes():
    """Return the available colour modes."""
    return tuple(_STYLE["modes"])


def set_mode(name):
    """Select the colour mode ('light' or 'dark') for figures built afterwards."""
    if name not in _STYLE["modes"]:
        raise StyleError(f"mode must be one of {', '.join(_STYLE['modes'])}")
    _MODE[0] = name


def mode():
    """Return the current colour mode."""
    return _MODE[0]


def _values(name):
    return _STYLE["modes"][_MODE[0]][name]


def color(name):
    """Return one named colour: surface, ink, muted, grid or fill."""
    if name not in ("surface", "ink", "muted", "grid", "fill"):
        raise StyleError(f"unknown colour name {name!r}")
    return _values(name)


def palette(n=None):
    """Return the first ``n`` categorical colours (all by default), in their fixed order."""
    colours = _values("categorical")
    if n is None:
        return list(colours)
    if isinstance(n, bool) or not isinstance(n, int) or not 1 <= n <= len(colours):
        raise StyleError(f"the validated categorical palette has {len(colours)} colours; "
                         "facet the figure when more groups are compared")
    return list(colours[:n])


def sequential():
    """Return the single-hue sequential ramp, from the surface towards high contrast."""
    return list(_values("sequential"))


def diverging():
    """Return the diverging ramp; the middle colour is the neutral midpoint."""
    return list(_values("diverging"))


def line(name):
    """Return a plotnine line size for a named weight (grid, edge, reference, model, interval)."""
    weights = _STYLE["lines_pt"]
    if name not in weights:
        raise StyleError(f"line weight must be one of {', '.join(weights)}")
    return weights[name] / _PT


def point_size():
    """Return the plotnine point size."""
    return _STYLE["point_size_mm"]


def fill_alpha():
    """Return the opacity of shaded intervals and ribbons."""
    return _STYLE["fill_alpha"]


def journals():
    """Return the journal presets (widths and heights in mm, type in pt)."""
    return json.loads(json.dumps(_STYLE["journals"]))


def _journal(journal):
    if journal is None:
        journal = _STYLE["default_journal"]
    if journal not in _STYLE["journals"]:
        raise StyleError(f"journal must be one of {', '.join(_STYLE['journals'])}")
    return _STYLE["journals"][journal]


def type_sizes(journal=None):
    """Return the journal's (base, small) type sizes in pt."""
    preset = _journal(journal)
    return float(preset["base_pt"]), float(preset["small_pt"])


def font(journal=None):
    """Return the first installed font of the journal's preferred list."""
    from matplotlib import font_manager
    installed = {entry.name for entry in font_manager.fontManager.ttflist}
    names = _STYLE["fonts"][_journal(journal)["fonts"]]
    return next((name for name in names if name in installed), "DejaVu Sans")


def _type(base_size, small_size, family):
    return theme(
        axis_title=element_text(size=base_size, family=family),
        axis_text=element_text(size=small_size, family=family),
        strip_text=element_text(size=base_size, family=family),
        legend_text=element_text(size=small_size, family=family),
        plot_title=element_text(size=base_size + 1, family=family),
    )


def theme_style(base_size=None, small_size=None, *, journal=None):
    """Surface-coloured theme with a recessive grid and the legend below the panel.

    Type sizes and the font follow the journal preset unless given.
    """
    preset = _journal(journal)
    base = float(preset["base_pt"] if base_size is None else base_size)
    small = float(preset["small_pt"] if small_size is None else small_size)
    if not 4 <= small <= base <= 16:
        raise StyleError("type sizes must satisfy 4 <= small_size <= base_size <= 16 pt")
    family = font(journal)
    ink, surface = color("ink"), color("surface")
    return theme_minimal(base_size=base, base_family=family) + theme(
        text=element_text(family=family, color=ink),
        plot_background=element_rect(fill=surface, color=surface),
        panel_background=element_rect(fill=surface, color=surface),
        panel_grid_minor=element_blank(),
        panel_grid_major=element_line(color=color("grid"), size=_STYLE["lines_pt"]["grid"]),
        panel_spacing=0.03,
        axis_title=element_text(color=ink),
        axis_text=element_text(color=color("muted")),
        strip_text=element_text(color=ink, weight="bold", ha="left"),
        strip_background=element_blank(),
        legend_position="bottom",
        legend_title=element_blank(),
        legend_box="vertical",
        legend_text=element_text(color=ink),
        legend_key_size=10,
        plot_title=element_text(color=ink, weight="bold", ha="left"),
    ) + _type(base, small, family)


def scale_color_categorical(**kwargs):
    """Categorical colour scale in the fixed palette order."""
    return scale_color_manual(values=_values("categorical"), **kwargs)


def scale_fill_categorical(**kwargs):
    """Categorical fill scale in the fixed palette order."""
    return scale_fill_manual(values=_values("categorical"), **kwargs)


def scale_fill_sequential(**kwargs):
    """Single-hue sequential fill for magnitudes."""
    return scale_fill_gradientn(colors=sequential(), **kwargs)


def scale_color_sequential(**kwargs):
    """Sequential colour for lines and points; omits the two steps nearest the surface."""
    return scale_color_gradientn(colors=sequential()[2:], **kwargs)


def scale_fill_diverging(**kwargs):
    """Diverging fill with a neutral midpoint; set ``limits`` symmetric about it."""
    return scale_fill_gradientn(colors=diverging(), **kwargs)


def size(fig, width_mm, height_mm):
    """Set the intended figure size in mm; ``save`` keeps this aspect ratio."""
    return fig + theme(figure_size=(width_mm / _MM, height_mm / _MM))


def _width(width, preset):
    names = {"single": "single_mm", "single-column": "single_mm", "double": "double_mm",
             "double-column": "double_mm"}
    if isinstance(width, str):
        if width not in names:
            raise StyleError("width must be 'single-column', 'double-column' or a number of mm")
        return float(preset[names[width]])
    if isinstance(width, bool) or not isinstance(width, (int, float)) or not 20 <= width <= preset["double_mm"]:
        raise StyleError(f"width in mm must be between 20 and the double-column width {preset['double_mm']:g}")
    return float(width)


def _figure_size(fig):
    plot = fig.last_plot if isinstance(fig, Compose) else fig
    value = plot.theme.getp("figure_size") if plot.theme is not None else None
    return value or (6.4, 4.8)


def save(fig, path, *, width=None, height=None, journal=None, formats=None, dpi=600,
         svg_text="paths"):
    """Write a figure as PDF, SVG and PNG at a journal column width.

    ``path`` is a file name; its suffix selects one format, and a name without
    a suffix writes ``formats`` (default PDF, SVG and PNG). ``width`` is
    'single-column', 'double-column' or a width in mm; by default figures
    designed for one column (up to 100 mm) use the single-column width and
    others the double-column width. ``height`` (mm) defaults to the figure's
    own aspect ratio, limited to the journal's maximum figure height. Type
    sizes and font follow the journal preset.

    PDF fonts are embedded as TrueType (Type 42) and the text stays editable.
    SVG text is drawn as glyph outlines by default (``svg_text='paths'``), so
    the file looks the same without the font; ``svg_text='text'`` keeps
    editable text that needs the font on the reader's computer. Creation
    dates and software names are omitted from file metadata, so a rerun with
    the same inputs, plotnine and Matplotlib versions and fonts gives the same
    files. Returns the paths.
    """
    if not isinstance(fig, (ggplot, Compose)):
        raise StyleError("save expects a plotnine figure")
    preset = _journal(journal)
    if isinstance(dpi, bool) or not isinstance(dpi, int) or not 150 <= dpi <= 2400:
        raise StyleError("dpi must be an integer in [150, 2400]")
    if svg_text not in ("paths", "text"):
        raise StyleError("svg_text must be 'paths' or 'text'")
    path = Path(path)
    known = ("pdf", "svg", "png")
    suffix = path.suffix.lower().lstrip(".")
    if suffix:
        if formats is not None:
            raise StyleError("give either a file suffix or formats=, not both")
        if suffix not in known:
            raise StyleError("file suffix must be .pdf, .svg or .png")
        formats, stem = (suffix,), path.with_suffix("")
    else:
        formats, stem = tuple(formats or known), path
        if not formats or any(f not in known for f in formats):
            raise StyleError("formats must be chosen from pdf, svg and png")
    w_in, h_in = _figure_size(fig)
    if width is None:
        width = "single-column" if w_in * _MM <= 100 else "double-column"
    width_mm = _width(width, preset)
    height_mm = min(width_mm * h_in / w_in, preset["max_height_mm"]) if height is None else height
    if (isinstance(height_mm, bool) or not isinstance(height_mm, (int, float))
            or not 15 <= height_mm <= preset["max_height_mm"]):
        raise StyleError(f"height must be between 15 and {preset['max_height_mm']:g} mm "
                         f"for {journal or _STYLE['default_journal']}")
    sizes = _type(preset["base_pt"], preset["small_pt"], font(journal))
    sized = (fig & sizes) if isinstance(fig, Compose) else (fig + sizes)
    sized = sized + theme(figure_size=(width_mm / _MM, height_mm / _MM))
    from matplotlib import pyplot, rc_context
    metadata = {"pdf": {"Creator": None, "Producer": None, "CreationDate": None},
                "svg": {"Creator": None, "Date": None},
                "png": {"Software": None}}
    rc = {"pdf.fonttype": 42, "ps.fonttype": 42,
          "svg.fonttype": "path" if svg_text == "paths" else "none",
          "svg.hashsalt": "figure", "pdf.compression": 6}
    written = []
    stem.parent.mkdir(parents=True, exist_ok=True)
    font_log = logging.getLogger("fontTools")
    level = font_log.level
    font_log.setLevel(logging.ERROR)  # font subsetting reports harmless table repairs
    try:
        with rc_context(rc), warnings.catch_warnings():
            warnings.filterwarnings("ignore", message=".*Saving.*")
            figure = sized.draw()
            try:
                for fmt in formats:
                    target = stem.with_suffix("." + fmt)
                    figure.savefig(target, format=fmt, dpi=dpi, metadata=metadata[fmt],
                                   facecolor=color("surface"))
                    written.append(target)
            finally:
                pyplot.close(figure)
    finally:
        font_log.setLevel(level)
    return written
