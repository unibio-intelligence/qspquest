"""Explicit paired-endpoint mapping for bounded native individual fitting."""

from copy import deepcopy
from dataclasses import dataclass
import json
import math

from . import CompiledSensitivities, QSPQuestError


def _fail(message):
    raise QSPQuestError({"code": "invalid_input", "message": message})


@dataclass(frozen=True)
class IndividualFitDataset:
    """Defensive request and source-row mapping; admission does not execute a fit."""

    _request_json: str
    _mapping_json: str

    @property
    def request(self):
        return json.loads(self._request_json)

    @property
    def mapping(self):
        return json.loads(self._mapping_json)

    def plan(self, source):
        """Admit against the supplied model without numerical integration."""
        request = self.request
        names = request["problem"]["request"]["objective"]["simulation"]["with_respect_to"]
        return CompiledSensitivities(source, names).plan_fit(request)

    def fit(self, source):
        """Return full native result, plan and exact provenance; inspect fit.status."""
        request = self.request
        names = request["problem"]["request"]["objective"]["simulation"]["with_respect_to"]
        model = CompiledSensitivities(source, names)
        plan = model.plan_fit(request)
        return {"result": model.fit(request), "plan": plan, "mapping": self.mapping}


def individual_fit_dataset(data, template, *, endpoints):
    """Map long-form rows to explicit native observation indices without sorting.

    Each row has exactly ``row`` (zero-based result row), ``endpoint`` (mapping
    key), and ``value`` (finite measured number in the endpoint's value_unit).
    Each endpoint declares exactly output, value_unit and a native Gaussian
    error. Native admission checks units and likelihood support later in plan()
    or fit(). No missing values, time alignment, unit inference or imputation.
    The template must be an individual request with empty observations and a
    fully explicit ordered observation grid; dosing and fitting controls persist.
    """
    if not isinstance(template, dict) or not isinstance(endpoints, dict) or not endpoints:
        _fail("template and nonempty endpoint mapping must be objects")
    request = deepcopy(template)
    try:
        if request["schema"] != "qspquest.fit/v0.1" or request["problem"]["kind"] != "individual":
            _fail("template requires qspquest.fit/v0.1 individual fitting")
        objective = request["problem"]["request"]["objective"]
        if objective["observations"] != []:
            _fail("template observation values must be empty")
        simulation = objective["simulation"]
        run = simulation["run"]
        if (simulation["schema"] != "qspquest.sensitivity/v0.1"
                or run["schema"] != "qspquest.run/v0.1"):
            _fail("template requires supported sensitivity and run schemas")
        regimen = run["regimen"]
        grid = regimen["observations"]
        if not isinstance(grid, list) or not grid or regimen.get("samples", []) != []:
            _fail("template needs a nonempty explicit observation grid and empty samples")
    except (KeyError, TypeError):
        _fail("malformed native individual template")
    if not isinstance(data, (list, tuple)) or not 1 <= len(data) <= 10_000:
        _fail("provide 1..10000 explicit observation records")
    outputs = set()
    for name, spec in endpoints.items():
        if not isinstance(name, str) or not name or not isinstance(spec, dict) or set(spec) != {"output", "value_unit", "error"}:
            _fail("endpoint entries require output, value_unit and native error")
        if not isinstance(spec["output"], str) or not spec["output"] or spec["output"] in outputs or not isinstance(spec["value_unit"], str) or not spec["value_unit"] or not isinstance(spec["error"], dict):
            _fail("endpoints require distinct named outputs, explicit units and errors")
        outputs.add(spec["output"])
    observations, mapping, seen, used = [], [], set(), set()
    for index, record in enumerate(data):
        if not isinstance(record, dict) or set(record) != {"row", "endpoint", "value"}:
            _fail(f"data[{index}] requires exactly row, endpoint and value")
        row, endpoint, value = record["row"], record["endpoint"], record["value"]
        if type(row) is not int or not 0 <= row < len(grid):
            _fail(f"data[{index}].row must index the explicit result grid")
        if not isinstance(endpoint, str) or endpoint not in endpoints:
            _fail(f"data[{index}].endpoint is not declared")
        try:
            finite = type(value) in (int, float) and math.isfinite(value)
        except OverflowError:
            finite = False
        if not finite:
            _fail(f"data[{index}].value must be a finite measured number")
        spec = endpoints[endpoint]
        key = (row, spec["output"])
        if key in seen:
            _fail(f"data[{index}] repeats a result-row/output key")
        seen.add(key)
        used.add(endpoint)
        observations.append({"row": row, "output": spec["output"],
                             "value": {"value": value, "unit": spec["value_unit"]},
                             "error": deepcopy(spec["error"])})
        mapping.append({"source_row": index, "result_row": row,
                        "endpoint": endpoint, "output": spec["output"],
                        "observation": deepcopy(grid[row])})
    if used != set(endpoints):
        _fail("every declared endpoint must have at least one measured row")
    objective["observations"] = observations
    try:
        return IndividualFitDataset(json.dumps(request, allow_nan=False),
                                    json.dumps(mapping, allow_nan=False))
    except (ValueError, TypeError):
        _fail("template or endpoint mapping contains nonfinite/non-JSON values")
