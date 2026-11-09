"""Run tutorial examples and compare their output with the text printed in each tutorial.

For each tutorial listed in TUTORIALS, the first `python docs/examples/... --out DIR`
command is run with a temporary output directory, and its standard output is
compared with the first ```text block that follows "prints:". Numbers are compared
with a tolerance at the printed precision; values below 1e-6 in magnitude, which
depend on the platform, need only both be small. The figure scripts are run with
QSPQUEST_FIGURE_DIR set to a temporary directory, so that their assertions are
checked without changing the committed figures.
Figure checks use the noninteractive Agg backend and a temporary writable
Matplotlib cache, independently of desktop plotting settings.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TUTORIALS = {
    "docs/tutorials/single-dose-pk.md": "docs/examples/single_dose_pk_figures.py",
    "docs/tutorials/multiple-dose-pk.md": "docs/examples/multiple_dose_pk_figures.py",
}
# Figure scripts for other tutorials. Each asserts its plotted data before drawing.
# Scripts that need a built CLI, MCP server or the validation repository
# (first-run, library-fitting, published-theophylline) are run manually.
FIGURE_SCRIPTS = [
    "docs/examples/oral_pk_analysis_figures.py",
    "docs/examples/parallel_predictive_checks_figures.py",
    "docs/examples/population_log_wald_figures.py",
    "docs/examples/profiled_proportional_errors_figures.py",
    "docs/examples/paired_endpoint_fitting_figures.py",
    "docs/examples/discrete_data_fitting_figures.py",
    "docs/examples/inverse_gaussian_input_figures.py",
]
NUMBER = re.compile(r"[-+]?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?")
SMALL = 1e-6


def expected(text):
    command = re.search(r"```sh\n(python docs/examples/\S+\.py --out \S+)\n```", text)
    printed = re.search(r"prints:\n\n```text\n(.*?)\n```", text, re.S)
    if not command or not printed:
        raise ValueError("tutorial has no runnable command and printed-output block")
    return command.group(1).split(), printed.group(1)


def tolerance(token):
    decimals = len(token.split(".")[1].split("e")[0].split("E")[0]) if "." in token else 0
    return 0.5 * 10 ** -decimals + 1e-12


def compare(printed, actual):
    """Return a list of differences between expected and actual output lines."""
    problems = []
    old_lines, new_lines = printed.strip().splitlines(), actual.strip().splitlines()
    if len(old_lines) != len(new_lines):
        return [f"line count {len(new_lines)} differs from documented {len(old_lines)}"]
    for number, (old, new) in enumerate(zip(old_lines, new_lines), 1):
        if NUMBER.sub("#", old) != NUMBER.sub("#", new):
            problems.append(f"line {number} text differs:\n  documented: {old}\n  actual:     {new}")
            continue
        for a, b in zip(NUMBER.findall(old), NUMBER.findall(new)):
            x, y = float(a), float(b)
            if abs(x) < SMALL and abs(y) < SMALL:
                continue
            if abs(x - y) > tolerance(a):
                problems.append(f"line {number}: documented {a}, actual {b}")
    return problems


def check(skip_figures, env):
    failures = []
    for page, figures in TUTORIALS.items():
        command, printed = expected((ROOT / page).read_text())
        with tempfile.TemporaryDirectory() as scratch:
            output = Path(scratch) / "output"
            args = [sys.executable] + command[1:command.index("--out") + 1] + [str(output)]
            run = subprocess.run(args, cwd=ROOT, env=env, capture_output=True, text=True)
            if run.returncode:
                failures.append(f"{page}: example failed (exit {run.returncode})\n{run.stderr[-2000:]}")
                continue
            failures.extend(f"{page}: {p}" for p in compare(printed, run.stdout))
            if not skip_figures:
                figure_env = {**env, "QSPQUEST_FIGURE_DIR": str(Path(scratch) / "figures")}
                draw = subprocess.run([sys.executable, figures], cwd=ROOT, env=figure_env,
                                      capture_output=True, text=True)
                if draw.returncode:
                    failures.append(f"{figures}: failed (exit {draw.returncode})\n{draw.stderr[-2000:]}")
        print(f"ran {page}")
    if not skip_figures:
        for script in FIGURE_SCRIPTS:
            with tempfile.TemporaryDirectory() as scratch:
                figure_env = {**env, "QSPQUEST_FIGURE_DIR": scratch}
                draw = subprocess.run([sys.executable, script], cwd=ROOT, env=figure_env,
                                      capture_output=True, text=True)
                if draw.returncode:
                    failures.append(f"{script}: failed (exit {draw.returncode})\n{draw.stderr[-2000:]}")
            print(f"ran {script}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        sys.exit(1)
    print(f"{len(TUTORIALS)} tutorials match their documented output")


def main(skip_figures=False):
    with tempfile.TemporaryDirectory(prefix="qspquest-matplotlib-") as cache:
        env = {**os.environ, "PYTHONPATH": str(ROOT / "docs/examples"),
               "MPLBACKEND": "Agg", "MPLCONFIGDIR": cache}
        check(skip_figures, env)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-figures", action="store_true",
                        help="do not run the figure scripts (they require the docs extra)")
    main(parser.parse_args().skip_figures)
