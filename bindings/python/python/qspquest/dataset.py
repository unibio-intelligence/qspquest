"""Explicit NONMEM-style row conversion for population fitting.

This module only builds versioned requests and preserves row provenance. The
Rust engine remains responsible for simulation, likelihoods, and estimation.
"""

from copy import deepcopy
from dataclasses import dataclass
import json
import math
import numbers

from . import QSPQuestError


def _fail(message):
    raise QSPQuestError({"code": "invalid_input", "message": message})


def _text(value, name):
    if not isinstance(value, str) or not value:
        _fail(f"{name} must be a nonempty string")
    return value


def _number(value, name):
    if isinstance(value, bool) or not isinstance(value, numbers.Real):
        _fail(f"{name} must be a finite number")
    try:
        if not math.isfinite(value):
            _fail(f"{name} must be a finite number")
    except OverflowError:
        _fail(f"{name} must be a finite number")
    return int(value) if isinstance(value, numbers.Integral) else float(value)


def _quantity(value, unit):
    return {"value": value, "unit": unit}


def _document(value, name):
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except ValueError as error:
            _fail(f"{name} is not valid JSON: {error}")
    if not isinstance(value, dict):
        _fail(f"{name} must be a request dict or JSON object")
    return deepcopy(value)


def _records(data):
    # pandas is optional. Its public to_dict API avoids a package dependency.
    if hasattr(data, "columns") and hasattr(data, "to_dict"):
        if len(data.columns) != len(set(data.columns)):
            _fail("data columns must be unique")
        data = data.to_dict(orient="records")
    if not isinstance(data, (list, tuple)) or not data:
        _fail("data must be a nonempty sequence of row dicts or a data frame")
    if any(not isinstance(row, dict) for row in data):
        _fail("every data row must be a dict")
    allowed = {
        "ID",
        "TIME",
        "EVID",
        "DV",
        "MDV",
        "AMT",
        "CMT",
        "RATE",
        "ADDL",
        "II",
        "SS",
    }
    for index, row in enumerate(data):
        if not {"ID", "TIME", "EVID"} <= row.keys():
            _fail(f"source row {index} requires ID, TIME, and EVID")
        extra = set(row) - allowed
        if extra:
            _fail(
                f"source row {index} has unsupported columns: {', '.join(sorted(extra))}"
            )
    return data


def _subject_id(value, index):
    if isinstance(value, bool) or value is None:
        _fail(f"source row {index} has an invalid ID")
    if isinstance(value, numbers.Real):
        value = _number(value, f"source row {index} ID")
        if isinstance(value, float) and value.is_integer():
            value = int(value)
    elif not isinstance(value, str):
        _fail(f"source row {index} has an invalid ID")
    identifier = str(value)
    if not identifier:
        _fail(f"source row {index} has an empty ID")
    return identifier


def _cmt(value, index):
    if isinstance(value, bool) or value is None:
        _fail(f"source row {index} requires CMT")
    if isinstance(value, numbers.Real):
        value = _number(value, f"source row {index} CMT")
        if isinstance(value, float) and value.is_integer():
            value = int(value)
    elif not isinstance(value, str):
        _fail(f"source row {index} has an invalid CMT")
    return str(value)


@dataclass(frozen=True)
class PopulationFitDataset:
    """A prepared request and zero-based source/result-row mapping."""

    request: dict
    subject_ids: tuple[str, ...]
    mapping: tuple[dict, ...]

    def to_subjects(self, *, covariates=None):
        """Prepare native-simulation analysis subjects with source-row provenance."""
        from .friendly import analysis_subjects

        return analysis_subjects(self, covariates=covariates)

    def fit(
        self, model, parameters, omega, error, *, covariates_by_subject=None, **options
    ):
        """Fit with an explicit shared analysis residual-error contract."""
        from .friendly import fit_population_dataset

        return fit_population_dataset(
            model,
            self,
            parameters,
            omega,
            error,
            covariates_by_subject=covariates_by_subject,
            **options,
        )


def population_fit_dataset(
    data,
    template,
    *,
    compartments,
    endpoints,
    time_unit,
    amount_unit,
    rate_unit=None,
    observation_side="post",
):
    """Build a FOCEI/SAEM fit request from an explicit NONMEM record subset.

    ``template`` is a population fit request with one empty subject objective.
    That subject supplies the sensitivity selection and run settings; records
    supply doses, observation times and values for every subject.
    """
    rows = _records(data)
    request = _document(template, "template")
    problem = request.get("problem")
    if (
        request.get("schema") != "qspquest.fit/v0.1"
        or not isinstance(problem, dict)
        or problem.get("kind") not in ("focei", "saem")
    ):
        _fail("template must be a FOCEI or SAEM qspquest.fit/v0.1 request")
    body = problem.get("request")
    subjects = body.get("subjects") if isinstance(body, dict) else None
    if (
        not isinstance(subjects, list)
        or len(subjects) != 1
        or not isinstance(subjects[0], dict)
    ):
        _fail("template must contain exactly one empty subject objective")
    objective = subjects[0]
    simulation = objective.get("simulation")
    run = simulation.get("run") if isinstance(simulation, dict) else None
    regimen = run.get("regimen") if isinstance(run, dict) else None
    if (
        not isinstance(simulation, dict)
        or simulation.get("schema") != "qspquest.sensitivity/v0.1"
    ):
        _fail("template requires a versioned sensitivity selection")
    if not isinstance(regimen, dict) or run.get("schema") != "qspquest.run/v0.1":
        _fail("template requires a versioned simulation run")
    if objective.get("observations") or objective.get("priors"):
        _fail("template subject observations and priors must be empty")
    for field in (
        "administrations",
        "resets",
        "covariate_changes",
        "observations",
        "samples",
        "checkpoints",
    ):
        if regimen.get(field):
            _fail(f"template regimen must have empty {field}")
    _text(time_unit, "time_unit")
    _text(amount_unit, "amount_unit")
    if rate_unit is not None:
        _text(rate_unit, "rate_unit")
    if observation_side not in ("pre", "post"):
        _fail(
            "observation_side must be pre or post; source-row order does not set dose-side semantics"
        )
    if not isinstance(compartments, dict) or not compartments:
        _fail("compartments must map dose CMT values to model targets")
    if not isinstance(endpoints, dict) or not endpoints:
        _fail(
            "endpoints must map observation CMT values to output, value_unit and error"
        )
    compartments = {
        str(key): _text(value, "compartment target")
        for key, value in compartments.items()
    }
    checked_endpoints = {}
    for key, endpoint in endpoints.items():
        if not isinstance(endpoint, dict) or set(endpoint) != {
            "output",
            "value_unit",
            "error",
        }:
            _fail("each endpoint requires exactly output, value_unit and error")
        _text(endpoint["output"], "endpoint output")
        _text(endpoint["value_unit"], "endpoint value_unit")
        error = endpoint["error"]
        if not isinstance(error, dict) or set(error) != {
            "additive_sd",
            "proportional_sd",
        }:
            _fail("endpoint error requires additive_sd and proportional_sd")
        if not isinstance(error["additive_sd"], dict) or set(error["additive_sd"]) != {
            "value",
            "unit",
        }:
            _fail("endpoint additive_sd requires an explicit quantity")
        _number(error["additive_sd"]["value"], "endpoint additive_sd")
        _text(error["additive_sd"]["unit"], "endpoint additive_sd unit")
        _number(error["proportional_sd"], "endpoint proportional_sd")
        checked_endpoints[str(key)] = deepcopy(endpoint)

    grouped = {}
    for index, row in enumerate(rows):
        identifier = _subject_id(row["ID"], index)
        time = _number(row["TIME"], f"source row {index} TIME")
        evid = _number(row["EVID"], f"source row {index} EVID")
        mdv = _number(row.get("MDV", 0), f"source row {index} MDV")
        if evid not in (0, 1) or mdv not in (0, 1):
            _fail(f"source row {index} supports only EVID 0/1 and MDV 0/1")
        amt = _number(row.get("AMT", 0), f"source row {index} AMT")
        rate = _number(row.get("RATE", 0), f"source row {index} RATE")
        addl = _number(row.get("ADDL", 0), f"source row {index} ADDL")
        ii = _number(row.get("II", 0), f"source row {index} II")
        ss = _number(row.get("SS", 0), f"source row {index} SS")
        if ss != 0 or rate < 0 or addl < 0 or addl > 4294967295 or int(addl) != addl:
            _fail(f"source row {index} has unsupported SS, RATE or ADDL")
        group = grouped.setdefault(identifier, {"doses": [], "observations": []})
        if evid == 1:
            if amt <= 0 or (addl and ii <= 0) or (not addl and ii != 0):
                _fail(
                    f"source row {index} requires positive AMT and positive II only with ADDL"
                )
            cmt = _cmt(row.get("CMT"), index)
            if cmt not in compartments:
                _fail(f"source row {index} dose CMT has no compartment mapping")
            if rate and rate_unit is None:
                _fail("positive RATE requires an explicit rate_unit")
            delivery = (
                {"kind": "bolus"}
                if not rate
                else {
                    "kind": "infusion",
                    "span": {"kind": "rate", "rate": _quantity(rate, rate_unit)},
                }
            )
            dose = {
                "target": compartments[cmt],
                "time": _quantity(time, time_unit),
                "amount": _quantity(amt, amount_unit),
                "delivery": delivery,
                "bioavailability": 1,
            }
            if addl:
                dose["repeat"] = {
                    "interval": _quantity(ii, time_unit),
                    "additional": int(addl),
                }
            group["doses"].append(dose)
        else:
            if amt or rate or addl or ii:
                _fail(f"source row {index} observation cannot carry dose fields")
            if mdv == 0:
                cmt = _cmt(row.get("CMT"), index)
                if cmt not in checked_endpoints:
                    _fail(f"source row {index} observation CMT has no endpoint mapping")
                dv = _number(row.get("DV"), f"source row {index} DV")
                group["observations"].append((index, time, cmt, dv))

    prepared_subjects = []
    mapping = []
    for subject_index, (identifier, group) in enumerate(grouped.items()):
        if not group["observations"]:
            _fail(f"subject {identifier} has no included observation rows")
        subject = deepcopy(objective)
        subject_run = subject["simulation"]["run"]
        subject_run["regimen"]["administrations"] = group["doses"]
        subject_run["regimen"]["observations"] = [
            {"time": _quantity(time, time_unit), "side": observation_side}
            for _, time, _, _ in group["observations"]
        ]
        subject_run["regimen"]["samples"] = []
        subject["observations"] = []
        for result_row, (source_row, _time, cmt, dv) in enumerate(
            group["observations"]
        ):
            endpoint = checked_endpoints[cmt]
            subject["observations"].append(
                {
                    "row": result_row,
                    "output": endpoint["output"],
                    "value": _quantity(dv, endpoint["value_unit"]),
                    "error": deepcopy(endpoint["error"]),
                }
            )
            mapping.append(
                {
                    "subject_index": subject_index,
                    "subject_id": identifier,
                    "source_row": source_row,
                    "observation_index": result_row,
                    "result_row": result_row,
                    "output": endpoint["output"],
                }
            )
        prepared_subjects.append(subject)
    body["subjects"] = prepared_subjects
    return PopulationFitDataset(request, tuple(grouped), tuple(mapping))
