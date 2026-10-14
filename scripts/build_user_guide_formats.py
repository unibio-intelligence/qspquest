#!/usr/bin/env python3
"""Build executable tutorial formats from the Markdown source guides.

Requires nbformat. Generation does not execute cells or install dependencies;
rerun verification before treating regenerated outputs as checked.
"""
from pathlib import Path
import json, re, hashlib
import nbformat
import argparse

parser = argparse.ArgumentParser(
    description="Generate the QSPQuest notebook and Rmd from Markdown guides."
)
parser.add_argument(
    "--output-dir",
    type=Path,
    help="Output directory; defaults to the checkout docs directory",
)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
output_dir = args.output_dir or root / "docs"
output_dir.mkdir(parents=True, exist_ok=True)
python_md = (root / "docs/PYTHON-USER-GUIDE.md").read_text()
nb = nbformat.v4.new_notebook()
nb.metadata = {
    "kernelspec": {"display_name": "Python 3", "language": "python", "name": "python3"},
    "language_info": {"name": "python"},
    "qspquest": {
        "guide_api": "0.1.0",
        "source_guide_sha256": hashlib.sha256(python_md.encode()).hexdigest(),
    },
}
python_md = python_md.replace(
    "Run the Python blocks in order in one session.",
    "Run the notebook cells from top to bottom with a kernel that has QSPQuest, pandas, and matplotlib installed. Installation commands below are reference instructions and are not executed by the notebook.",
)
python_md = python_md.replace(
    "optional plotting dependencies.",
    "optional plotting dependencies. This notebook includes the plotting example and keeps its rendered output.",
)
pattern = re.compile(r"```python\n(.*?)\n```", re.S)
cells = []
position = 0
inserted_setup = False
for match in pattern.finditer(python_md):
    prose = python_md[position : match.start()].strip()
    if prose:
        cells.append(nbformat.v4.new_markdown_cell(prose))
    if not inserted_setup:
        cells.append(
            nbformat.v4.new_markdown_cell(
                "### Notebook execution setup\n\nRun the cells in order. Generated files go into `qspquest-python-tutorial-output` beneath the directory where the notebook kernel starts. Package installation is a separate prerequisite."
            )
        )
        cells.append(
            nbformat.v4.new_code_cell(
                'from pathlib import Path\nimport os\n\nanalysis_directory = Path.cwd() / "qspquest-python-tutorial-output"\nanalysis_directory.mkdir(exist_ok=True)\nos.chdir(analysis_directory)\nprint("Analysis files are saved in qspquest-python-tutorial-output.")'
            )
        )
        inserted_setup = True
    code = match.group(1)
    if 'plt.savefig("trajectory.png", dpi=150)' in code:
        code = code.replace("plt.close()", "plt.show()\nplt.close()")
    cells.append(nbformat.v4.new_code_cell(code))
    position = match.end()
remainder = python_md[position:].strip()
if remainder:
    cells.append(nbformat.v4.new_markdown_cell(remainder))
nb.cells = cells
nbformat.write(nb, output_dir / "PYTHON-USER-GUIDE.ipynb")

r_md = (root / "docs/R-USER-GUIDE.md").read_text()
r_md = r_md.split("\n", 1)[1].lstrip()
r_md = r_md.replace(
    "Run the R blocks in order in one session.",
    "Render this R Markdown file after installing QSPQuest, jsonlite, knitr, and rmarkdown. Installation chunks are displayed but are not executed; all analysis chunks execute in order.",
)
r_md = r_md.replace("## Contents\n", "## Contents\n", 1)
# The generated HTML provides a native navigable table of contents.
r_md = re.sub(
    r"## Contents\n.*?(?=## Install and verify)", "", r_md, count=1, flags=re.S
)
index = 0


def r_chunk(match):
    global index
    index += 1
    code = match.group(1)
    reference = code.startswith("install.packages") or code.startswith(".libPaths")
    option = ", eval=FALSE" if reference else ""
    return "```{r example-" + str(index) + option + "}\n" + code + "\n```"


r_md = re.sub(r"```r\n(.*?)\n```", r_chunk, r_md, flags=re.S)
header = """---
title: "QSPQuest quickstart and user guide for R"
output:
  html_document:
    toc: true
    toc_depth: 3
    toc_float: true
    code_folding: show
    self_contained: true
    df_print: paged
---

```{r setup, include=FALSE}
knitr::opts_chunk$set(echo = TRUE, error = FALSE, warning = TRUE,
                      message = TRUE, fig.width = 7, fig.height = 4.5,
                      fig.align = "center")
# Pass an installed QSPQuest library through R_LIBS_USER or .libPaths()
# before rendering; this tutorial does not install packages automatically.
analysis_directory <- file.path(getwd(), "qspquest-r-tutorial-output")
dir.create(analysis_directory, showWarnings = FALSE)
knitr::opts_knit$set(root.dir = normalizePath(analysis_directory))
```

Generated analysis files are saved in `qspquest-r-tutorial-output` alongside
the Rmd file. The plot, tables and fit statuses appear in the rendered HTML.
Use `rmarkdown::render("R-USER-GUIDE.Rmd")` to rerun the full analysis.

"""
(output_dir / "R-USER-GUIDE.Rmd").write_text(header + r_md)
print(
    "Created notebook:",
    len(nb.cells),
    "cells;",
    sum(c.cell_type == "code" for c in nb.cells),
    "code cells",
)
print("Created Rmd:", index, "example chunks; installation chunks marked eval=FALSE")
