"""Shared validation, likelihoods and aggregate budgets for native analyses."""

from copy import deepcopy
import hashlib
import json
import math
from pathlib import Path
import sys
import threading


def dependencies():
    try:
        import numpy as np
        from scipy import optimize, special, stats
    except ImportError as error:
        raise ImportError(
            "Install qspquest[analysis] to use analysis workflows"
        ) from error
    return np, optimize, special, stats


def fail(message, code="invalid_input"):
    from . import QSPQuestError

    raise QSPQuestError({"code": code, "message": message})


def finite(value, name):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        fail(f"{name} must be a finite number")
    return float(value)


def integer(value, name, lower, upper):
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or not lower <= value <= upper
    ):
        fail(f"{name} must be an integer in [{lower}, {upper}]")
    return value


def identity(model, inputs):
    np, _, _, _ = dependencies()
    import scipy

    def numeric_json(value):
        if isinstance(value, np.ndarray):
            return value.tolist()
        if isinstance(value, np.integer):
            return int(value)
        if isinstance(value, np.floating):
            return float(value)
        raise TypeError("analysis inputs must be JSON-compatible quantities and arrays")

    payload = json.dumps(
        inputs,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
        default=numeric_json,
    )
    source = hashlib.sha256()
    for path in sorted(Path(__file__).parent.glob("*.py")):
        source.update(path.name.encode())
        source.update(path.read_bytes())
    return {
        "model_content_hash": model.model_content_hash,
        "analysis_request_hash": "sha256:"
        + hashlib.sha256(payload.encode()).hexdigest(),
        "analysis_version": "qspquest.analysis/0.1",
        "analysis_source_sha256": "sha256:" + source.hexdigest(),
        "python_version": sys.version.split()[0],
        "numpy_version": np.__version__,
        "scipy_version": scipy.__version__,
    }


class Budget:
    def __init__(self, maximum, control=None):
        self.maximum = integer(maximum, "max_simulations", 1, 1000000)
        self.used = 0
        self.control = control
        self._lock = threading.Lock()

    def run(self, model, request):
        if self.control is not None:
            self.control.check()
        # Charge a dispatched native call exactly once, including failed calls.
        # The lock covers accounting only; independent executions can overlap.
        with self._lock:
            if self.used >= self.maximum:
                fail("aggregate native simulation budget exhausted", "budget")
            self.used += 1
        return model.run(request)


def parameter_specs(parameters):
    if not isinstance(parameters, dict) or not 1 <= len(parameters) <= 16:
        fail("parameters must contain 1..16 named bounds")
    specs = deepcopy(parameters)
    paths = set()
    for name, item in specs.items():
        if not isinstance(name, str) or not name or not isinstance(item, dict):
            fail("parameter names and specifications are required")
        if set(item) - {"initial", "lower", "upper", "unit", "path"}:
            fail(f"unknown parameter fields for {name}")
        for key in ("initial", "lower", "upper"):
            item[key] = finite(item.get(key), f"{name}.{key}")
        if (
            not item["lower"] < item["upper"]
            or not item["lower"] <= item["initial"] <= item["upper"]
        ):
            fail(f"invalid initial value or bounds for {name}")
        if not isinstance(item.get("unit"), str) or not item["unit"]:
            fail(f"{name} requires a declared unit")
        path = item.get("path")
        if path is not None:
            # Request-time event controls use quantity-valued JSON-pointer paths.
            if (
                not isinstance(path, list)
                or len(path) < 4
                or path[:2] != ["regimen", "administrations"]
            ):
                fail("event parameter paths must select an administration quantity")
            if isinstance(path[2], bool) or not isinstance(path[2], int) or path[2] < 0:
                fail("event parameter path requires an administration index")
            if path[3:] not in (
                ["time"],
                ["lag"],
                ["delivery", "span", "duration"],
                ["repeat", "interval"],
            ):
                fail(
                    "only administration time, lag, duration or repeat interval may be fitted"
                )
            key = tuple(path)
        else:
            key = ("parameters", name)
        if key in paths:
            fail("parameter paths must be distinct")
        paths.add(key)
    return specs


def bind(run, specs, values):
    result = deepcopy(run)
    for name, value in values.items():
        item = specs[name]
        q = {"value": finite(value, name), "unit": item["unit"]}
        path = item.get("path")
        if path is None:
            result.setdefault("parameters", {})[name] = q
        else:
            try:
                parent = result
                for component in path[:-1]:
                    parent = parent[component]
                parent[path[-1]] = q
            except (KeyError, IndexError, TypeError):
                fail(f"event path for {name} does not exist")
    return result


def columns(result):
    np, _, _, _ = dependencies()
    return {
        c["name"]: (np.asarray(c["values"], dtype=float), c["unit"])
        for c in result["outputs"]
    }


def observations(result, rows):
    np, _, _, _ = dependencies()
    outputs = columns(result)
    if not isinstance(rows, list) or not 1 <= len(rows) <= 10000:
        fail("observations must contain 1..10000 rows")
    y, prediction, censored, keys = [], [], [], set()
    for observation in rows:
        row, output = observation.get("row"), observation.get("output")
        integer(row, "observation.row", 0, len(result["times"]) - 1)
        if output not in outputs:
            fail(f"unknown observation output {output}")
        if (row, output) in keys:
            fail("duplicate observation row and output")
        keys.add((row, output))
        value = observation.get("value", {})
        if value.get("unit") != outputs[output][1]:
            fail("observation units must equal the declared output units")
        y.append(finite(value.get("value"), "observation.value"))
        prediction.append(float(outputs[output][0][row]))
        censoring = observation.get("censoring")
        if censoring not in (None, "left"):
            fail("only left censoring is supported")
        censored.append(censoring == "left")
    return np.asarray(y), np.asarray(prediction), np.asarray(censored)


def error_spec(error):
    if not isinstance(error, dict) or set(error) - {
        "model",
        "additive",
        "proportional",
    }:
        fail("error must specify model, additive and/or proportional SD")
    result = deepcopy(error)
    result.setdefault("model", "normal")
    if result["model"] not in ("normal", "lognormal"):
        fail("error.model must be normal or lognormal")
    for name in ("additive", "proportional"):
        value = result.get(name, 0.0)
        if isinstance(value, dict):
            if set(value) != {"initial", "lower", "upper"}:
                fail(f"fitted {name} requires initial, lower and upper")
            for key in value:
                value[key] = finite(value[key], f"error.{name}.{key}")
            if (
                not 0 < value["lower"] <= value["initial"] <= value["upper"]
                or value["lower"] == value["upper"]
            ):
                fail("fitted SD bounds must be strictly positive with nonzero width")
        else:
            result[name] = finite(value, f"error.{name}")
            if result[name] < 0:
                fail("residual SD cannot be negative")
    if result["model"] == "lognormal" and result["additive"] != 0:
        fail("lognormal error requires zero additive SD")
    if all(result[k] == 0 for k in ("additive", "proportional")):
        fail("at least one residual SD must be positive")
    return result


def initial_error(error):
    return {k: (v["initial"] if isinstance(v, dict) else v) for k, v in error.items()}


def likelihood(y, prediction, censored, error):
    np, _, special, _ = dependencies()
    if not np.all(np.isfinite(prediction)):
        fail("prediction must be finite", "domain")
    if error["model"] == "lognormal":
        if np.any(y <= 0) or np.any(prediction <= 0):
            fail(
                "lognormal observations, limits and predictions must be positive",
                "domain",
            )
        residual = np.log(y) - np.log(prediction)
        sd = np.full(len(y), error["proportional"])
        jacobian = np.log(y)
    else:
        residual = y - prediction
        sd = np.sqrt(error["additive"] ** 2 + (error["proportional"] * prediction) ** 2)
        jacobian = np.zeros(len(y))
    if np.any(sd <= 0) or not np.all(np.isfinite(sd)):
        fail("residual variance must be positive at every observation", "domain")
    z = residual / sd
    terms = 0.5 * z * z + np.log(sd) + 0.5 * math.log(2 * math.pi) + jacobian
    terms[censored] = -special.log_ndtr(z[censored])
    if not np.all(np.isfinite(terms)):
        fail("likelihood is not finite", "domain")
    return float(terms.sum()), residual, sd


def scalar_parameters(specs, values):
    return {
        name: {"value": float(values[name]), "unit": item["unit"]}
        for name, item in specs.items()
    }
