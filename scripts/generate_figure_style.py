"""Write figure style definitions for Python, R, CSS and JavaScript from spec/style/figure-style.json.

Outputs: the Python and R package copies, CSS custom properties (light and dark
modes) and an ES module with D3-ready colour schemes plus its type declarations.
Use --check to verify that every output matches the style file.
"""

import argparse
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "spec/style/figure-style.json"
COPIES = ("bindings/python/python/qspquest/plot/figure-style.json", "bindings/r/inst/style/figure-style.json")
HEX = re.compile(r"^#[0-9A-F]{6}$")
HEADER = "Generated from spec/style/figure-style.json by scripts/generate_figure_style.py. Do not edit."
SCALARS = ("surface", "ink", "muted", "grid", "fill")
SCHEMES = {"categorical": 3, "sequential": 6, "diverging": 5}


def validate(definitions):
    if definitions.get("schema") != "qspquest.figure-style/v1":
        raise ValueError("unknown figure style schema")
    if set(definitions["modes"]) != {"light", "dark"}:
        raise ValueError("the style needs light and dark modes")
    for mode, values in definitions["modes"].items():
        if set(values) != set(SCALARS) | set(SCHEMES):
            raise ValueError(f"{mode} mode has unexpected or missing entries")
        for name in SCALARS:
            if not HEX.match(values[name]):
                raise ValueError(f"{mode}.{name} must be an upper-case #RRGGBB colour")
        for name, count in SCHEMES.items():
            if len(values[name]) != count or not all(HEX.match(c) for c in values[name]):
                raise ValueError(f"{mode}.{name} must hold {count} #RRGGBB colours")
    for name, journal in definitions["journals"].items():
        if journal["fonts"] not in definitions["fonts"] or not journal["small_pt"] <= journal["base_pt"]:
            raise ValueError(f"journal preset {name} is inconsistent")
    if definitions["default_journal"] not in definitions["journals"]:
        raise ValueError("default journal is not a preset")
    return definitions


def css(definitions):
    def block(values, indent):
        lines = [f"{indent}--figure-{name}: {values[name]};" for name in SCALARS]
        for name in SCHEMES:
            lines += [f"{indent}--figure-{name}-{i}: {colour};"
                      for i, colour in enumerate(values[name], 1)]
        return "\n".join(lines)

    light, dark = definitions["modes"]["light"], definitions["modes"]["dark"]
    return (f"/* {HEADER} */\n:root {{\n{block(light, '  ')}\n}}\n\n"
            f"@media (prefers-color-scheme: dark) {{\n  :root:not([data-theme=\"light\"]) {{\n"
            f"{block(dark, '    ')}\n  }}\n}}\n\n"
            f":root[data-theme=\"dark\"] {{\n{block(dark, '  ')}\n}}\n")


def module(definitions):
    def frozen(name):
        values = {mode: definitions["modes"][mode][name] for mode in ("light", "dark")}
        return f"export const {name} = Object.freeze({json.dumps(values)});"

    colours = {mode: {k: definitions["modes"][mode][k] for k in SCALARS} for mode in ("light", "dark")}
    return "\n".join([
        f"// {HEADER}",
        "// Schemes are arrays usable with D3, for example",
        "// d3.scaleOrdinal(schemeCategorical.light) or",
        "// d3.scaleSequential(d3.interpolateRgbBasis(schemeSequential.light)).",
        f"export const style = Object.freeze({json.dumps(definitions)});",
        f"export const colors = Object.freeze({json.dumps(colours)});",
        frozen("categorical").replace("const categorical", "const schemeCategorical"),
        frozen("sequential").replace("const sequential", "const schemeSequential"),
        frozen("diverging").replace("const diverging", "const schemeDiverging"),
        "",
    ])


def declarations():
    return "\n".join([
        f"// {HEADER}",
        'export type Mode = "light" | "dark";',
        "export interface ModeColors { surface: string; ink: string; muted: string; grid: string; fill: string; }",
        "export declare const style: Readonly<Record<string, unknown>>;",
        "export declare const colors: Readonly<Record<Mode, ModeColors>>;",
        "export declare const schemeCategorical: Readonly<Record<Mode, readonly string[]>>;",
        "export declare const schemeSequential: Readonly<Record<Mode, readonly string[]>>;",
        "export declare const schemeDiverging: Readonly<Record<Mode, readonly string[]>>;",
        "",
    ])


def outputs(root):
    text = (root / SOURCE).read_text()
    definitions = validate(json.loads(text))
    files = {copy: text for copy in COPIES}
    files["spec/style/figure-style.css"] = css(definitions)
    files["spec/style/figure-style.mjs"] = module(definitions)
    files["spec/style/figure-style.d.mts"] = declarations()
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        files = outputs(args.root)
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, f"figure style: {error}\n")
    stale = []
    for name, content in files.items():
        path = args.root / name
        if args.check:
            if not path.is_file() or path.read_text() != content:
                stale.append(name)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    if stale:
        parser.exit(1, "stale style outputs: " + ", ".join(stale) + "\n")
    print(json.dumps({"status": "matched" if args.check else "generated", "files": len(files)}))


if __name__ == "__main__":
    sys.exit(main())
