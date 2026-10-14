"""Validated SBML rate-rule ODEs and the bounded QSPQuest OMEX profile.

Install ``qspquest[interchange]``. General reactions, SBML events and arbitrary
SED-ML are rejected, never silently discarded. See the interchange scope guide.
"""

from copy import deepcopy
from dataclasses import dataclass, field
from hashlib import sha256
from io import BytesIO
import json
import math
from pathlib import Path, PurePosixPath
import re
import stat
import xml.etree.ElementTree as ET
import zipfile

PROFILE = "qspquest.omex/v1"
FIT_PROFILE = "qspquest.omex.individual-fit/v1"
SBML_PROFILE = "qspquest.sbml.rate-rules/v1"
NS = "https://qspquest.org/ns/interchange/v1"
SEDML_NS = "http://sed-ml.org/sed-ml/level1/version5"
MANIFEST_NS = "http://identifiers.org/combine.specifications/omex-manifest"
MAX_MEMBER_BYTES = 10_000_000
MAX_ARCHIVE_BYTES = 50_000_000
MAX_EXPANDED_BYTES = 200_000_000


class InterchangeError(ValueError):
    """A parse, validation, integrity or unsupported-semantics failure.

    ``code`` distinguishes missing_dependency, invalid_document, unsupported,
    integrity and resource_limit. No model or partial trajectory is returned.
    """

    def __init__(self, code, message):
        self.code = code
        super().__init__(message)


def _fail(code, message):
    raise InterchangeError(code, message)


def _libsbml():
    try:
        import libsbml
    except ImportError as exc:
        raise InterchangeError(
            "missing_dependency",
            "libSBML could not load ("
            + str(exc)
            + "). Install qspquest[interchange] and required system libraries; minimal Debian/Ubuntu needs libexpat1.",
        ) from exc
    return libsbml


def _json(value):
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode()


def _load_json(data):
    def unique(pairs):
        result = {}
        for name, value in pairs:
            if name in result:
                _fail("invalid_document", "Duplicate JSON key: " + name)
            result[name] = value
        return result

    def nonfinite(value):
        _fail("invalid_document", "Nonfinite JSON constant: " + value)

    return json.loads(data, object_pairs_hook=unique, parse_constant=nonfinite)


def _xml(data):
    if isinstance(data, str):
        data = data.encode()
    if not data or len(data) > MAX_MEMBER_BYTES:
        _fail("resource_limit", "XML must contain between 1 byte and 10 MB.")
    try:
        decoded = data.decode("utf-8")
    except UnicodeError as exc:
        raise InterchangeError("unsupported", "XML must use UTF-8 encoding.") from exc
    if "<!DOCTYPE" in decoded.upper() or "<!ENTITY" in decoded.upper():
        _fail("unsupported", "DTD and entity declarations are not supported.")
    try:
        root = ET.fromstring(decoded)
        pending = [(root, 0)]
        count = 0
        while pending:
            element, depth = pending.pop()
            count += 1
            if depth > 100 or count > 100000:
                _fail(
                    "resource_limit",
                    "XML nesting or element count exceeds profile limits.",
                )
            pending.extend((c, depth + 1) for c in element)
        return root
    except ET.ParseError as exc:
        raise InterchangeError(
            "invalid_document", "Malformed XML: " + str(exc)
        ) from exc


def _signature(element):
    return (
        element.tag,
        sorted(element.attrib.items()),
        (element.text or "").strip(),
        [_signature(c) for c in element],
    )


def _read_sbml(text):
    _xml(text)
    lib = _libsbml()
    doc = lib.readSBMLFromString(text)
    doc.checkConsistency()
    errors = [
        doc.getError(i).getMessage().strip()
        for i in range(doc.getNumErrors())
        if doc.getError(i).getSeverity() >= lib.LIBSBML_SEV_ERROR
    ]
    if errors:
        _fail(
            "invalid_document", "libSBML validation failed: " + " | ".join(errors[:5])
        )
    if doc.getModel() is None:
        _fail("invalid_document", "SBML has no model.")
    if any(
        doc.getPlugin(i).getPackageName() != "l3v2extendedmath"
        for i in range(doc.getNumPlugins())
    ):
        _fail("unsupported", "SBML Level 3 extension packages are outside this subset.")
    return doc


def validate_sbml(text):
    """Return libSBML consistency warnings; raise for validation errors.

    This validates SBML, not whether its executable semantics are supported.
    """
    doc = _read_sbml(text)
    return {
        "status": "valid",
        "libsbml": _libsbml().getLibSBMLDottedVersion(),
        "warnings": [
            doc.getError(i).getMessage().strip() for i in range(doc.getNumErrors())
        ],
    }


def _expr_formula(expr):
    kind = expr["kind"]
    if kind == "literal":
        value = expr["value"]
        return (
            ("true" if value else "false")
            if isinstance(value, bool)
            else repr(float(value))
        )
    if kind == "symbol":
        return "time" if expr["name"] == "time" else expr["name"]
    args = [_expr_formula(a) for a in expr.get("arguments", [])]
    if kind == "unary" and expr["operator"] in ("positive", "negative"):
        return ("+" if expr["operator"] == "positive" else "-") + "(" + args[0] + ")"
    if kind == "binary" and expr["operator"] in (
        "add",
        "subtract",
        "multiply",
        "divide",
        "power",
    ):
        op = {
            "add": "+",
            "subtract": "-",
            "multiply": "*",
            "divide": "/",
            "power": "^",
        }[expr["operator"]]
        return "(" + args[0] + op + args[1] + ")"
    if kind == "call" and expr["name"] in (
        "exp",
        "log",
        "sqrt",
        "sin",
        "cos",
        "tan",
        "tanh",
        "abs",
        "min",
        "max",
        "floor",
        "ceil",
    ):
        name = {"log": "ln", "ceil": "ceiling"}.get(expr["name"], expr["name"])
        return name + "(" + ",".join(args) + ")"
    _fail("unsupported", "Expression is outside the SBML export subset: " + kind)


def _math(expr, lib):
    node = lib.parseL3Formula(_expr_formula(expr))
    if node is None:
        _fail("invalid_document", "Expression cannot be represented as SBML MathML.")
    return node


def export_sbml(model):
    """Export native model JSON or ImportedModel to SBML Level 3 Version 2.

    The annotated numeric-coordinate profile preserves native units, bounds and
    dosing declarations. SBML quantities themselves use numeric coordinates;
    this is not a claim of portable physical unit inference by other solvers.
    Text .qsp, linear_pk and event/history/covariate features are unsupported.
    """
    from . import CompiledModel, _call, _native

    source = deepcopy(model.model if isinstance(model, ImportedModel) else model)
    if not isinstance(source, dict):
        _fail("unsupported", "export_sbml requires native model JSON or ImportedModel.")
    for name in (
        "linear_pk",
        "covariates",
        "individual",
        "switches",
        "dose_history",
        "dose_switches",
        "invariants",
    ):
        if source.get(name):
            _fail("unsupported", "SBML export does not support " + name + ".")
    if any(
        s.get("dosing", {}).get("lag")
        for s in source.get("states", [])
        if s.get("dosing")
    ):
        _fail(
            "unsupported",
            "Model-level dosing lag is outside this export subset; use request-level lag.",
        )
    CompiledModel(source)
    # Export the same numerical coordinates evaluated by the unit-aware native
    # compiler, including mg/g, mL/L and minute/hour scale conversions.
    units = {p["name"]: p["default"]["unit"] for p in source["parameters"]}
    units.update({s["name"]: s["unit"] for s in source["states"]})
    units["time"] = source["time_unit"]
    for s in source["states"]:
        if s.get("dosing"):
            units["input_" + s["name"]] = (
                "(" + s["unit"] + ")/(" + source["time_unit"] + ")"
            )
    for d in source.get("definitions", []):
        if not d.get("unit"):
            _fail(
                "unsupported",
                "Export requires explicit units on algebraic definitions.",
            )
        units[d["name"]] = d["unit"]

    def normalized(expression, unit):
        return json.loads(
            _call(
                _native.normalize_interchange_expression,
                _json(expression).decode(),
                _json(units).decode(),
                unit,
            )
        )

    lib = _libsbml()
    doc = lib.SBMLDocument(3, 2)
    target = doc.createModel()
    target.setId(source["model_id"])
    if not target.getId():
        _fail("unsupported", "SBML requires an SId-compatible model identifier.")
    # Physical time is explicit; other coordinates carry original units in the
    # native document annotation, rather than pretending they are SI amounts.
    seconds = {"s": 1.0, "min": 60.0, "h": 3600.0, "day": 86400.0, "d": 86400.0}.get(
        source["time_unit"]
    )
    if seconds is None:
        _fail("unsupported", "SBML export time_unit must be s, min, h or day.")
    unit = target.createUnitDefinition()
    unit.setId("pfx_time_unit")
    item = unit.createUnit()
    item.setKind(lib.UNIT_KIND_SECOND)
    item.setExponent(1)
    item.setScale(0)
    item.setMultiplier(seconds)
    target.setTimeUnits("pfx_time_unit")
    target.setSubstanceUnits("dimensionless")
    target.setExtentUnits("dimensionless")
    annotation = ET.Element("{" + NS + "}nativeModel", {"profile": SBML_PROFILE})
    annotation.text = _json(source).decode()
    target.appendAnnotation(ET.tostring(annotation, encoding="unicode"))
    compartment = target.createCompartment()
    compartment.setId("pfx_compartment")
    compartment.setSize(1)
    compartment.setConstant(True)
    compartment.setSpatialDimensions(0)
    names = {p["name"] for p in source["parameters"]} | {
        s["name"] for s in source["states"]
    }
    names |= {d["name"] for d in source.get("definitions", [])} | {
        o["name"] for o in source["outputs"]
    }
    if names & {"pfx_compartment", "pfx_time_unit", "time"} or any(
        not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", n) for n in names
    ):
        _fail(
            "unsupported",
            "Model names collide with SBML profile identifiers or are not SIds.",
        )
    for p in source["parameters"]:
        parameter = target.createParameter()
        parameter.setId(p["name"])
        parameter.setValue(p["default"]["value"])
        parameter.setConstant(True)
    for s in source["states"]:
        if s["initial"]["kind"] != "quantity":
            _fail("unsupported", "SBML export requires literal initial quantities.")
        species = target.createSpecies()
        species.setId(s["name"])
        species.setCompartment("pfx_compartment")
        quantity = s["initial"]["quantity"]
        species.setInitialAmount(
            _call(
                _native.convert_interchange_quantity,
                quantity["value"],
                quantity["unit"],
                s["unit"],
            )
        )
        species.setHasOnlySubstanceUnits(True)
        species.setBoundaryCondition(False)
        species.setConstant(False)
        if s.get("dosing"):
            input_id = "input_" + s["name"]
            if input_id in names:
                _fail("unsupported", "Dosing input identifier collision: " + input_id)
            parameter = target.createParameter()
            parameter.setId(input_id)
            parameter.setValue(0)
            parameter.setConstant(True)
        rule = target.createRateRule()
        rule.setVariable(s["name"])
        rule.setMath(
            _math(
                normalized(
                    s["rhs"], "(" + s["unit"] + ")/(" + source["time_unit"] + ")"
                ),
                lib,
            )
        )
    for p in source.get("definitions", []) + source["outputs"]:
        parameter = target.createParameter()
        parameter.setId(p["name"])
        parameter.setConstant(False)
        rule = target.createAssignmentRule()
        rule.setVariable(p["name"])
        rule.setMath(_math(normalized(p["expression"], p["unit"]), lib))
    text = lib.writeSBMLToString(doc)
    validate_sbml(text)
    return text


def _ast(node, lib, names):
    if node is None:
        _fail("invalid_document", "Missing MathML expression.")
    if node.isInteger() or node.isReal() or node.isRational():
        value = float(node.getValue())
        if not math.isfinite(value):
            _fail("invalid_document", "MathML numbers must be finite.")
        return {"kind": "literal", "value": value}
    if node.getType() == lib.AST_NAME_TIME:
        return {
            "kind": "binary",
            "operator": "divide",
            "arguments": [
                {"kind": "symbol", "name": "time"},
                {"kind": "symbol", "name": "pfx_coordinate_time"},
            ],
        }
    if node.isName():
        if node.getName() not in names:
            _fail(
                "unsupported",
                "Undefined or unsupported MathML symbol: " + node.getName(),
            )
        return {"kind": "symbol", "name": node.getName()}
    args = [_ast(node.getChild(i), lib, names) for i in range(node.getNumChildren())]
    operators = {
        lib.AST_PLUS: "add",
        lib.AST_MINUS: "subtract",
        lib.AST_TIMES: "multiply",
        lib.AST_DIVIDE: "divide",
        lib.AST_POWER: "power",
        lib.AST_FUNCTION_POWER: "power",
    }
    if node.getType() in operators:
        op = operators[node.getType()]
        if op == "subtract" and len(args) == 1:
            return {"kind": "unary", "operator": "negative", "arguments": args}
        if len(args) < 2:
            _fail("unsupported", "Unsupported MathML operator arity.")
        result = args[0]
        for arg in args[1:]:
            result = {"kind": "binary", "operator": op, "arguments": [result, arg]}
        return result
    functions = {
        lib.AST_FUNCTION_EXP: "exp",
        lib.AST_FUNCTION_LN: "log",
        lib.AST_FUNCTION_SIN: "sin",
        lib.AST_FUNCTION_COS: "cos",
        lib.AST_FUNCTION_TAN: "tan",
        lib.AST_FUNCTION_TANH: "tanh",
        lib.AST_FUNCTION_ABS: "abs",
        lib.AST_FUNCTION_MIN: "min",
        lib.AST_FUNCTION_MAX: "max",
        lib.AST_FUNCTION_FLOOR: "floor",
        lib.AST_FUNCTION_CEILING: "ceil",
    }
    if node.getType() in functions:
        return {"kind": "call", "name": functions[node.getType()], "arguments": args}
    if (
        node.getType() == lib.AST_FUNCTION_ROOT
        and len(args) == 2
        and args[0] == {"kind": "literal", "value": 2.0}
    ):
        return {"kind": "call", "name": "sqrt", "arguments": [args[1]]}
    _fail(
        "unsupported",
        "Unsupported MathML construct: "
        + (lib.formulaToL3String(node) or str(node.getType())),
    )


@dataclass
class ImportedModel:
    """Native model JSON, source digest and explicit semantic/unit warnings."""

    model: dict
    source_sha256: str
    warnings: tuple = ()

    def compile(self):
        """Compile using the installed native engine; no alternate solver fallback."""
        from . import CompiledModel

        return CompiledModel(deepcopy(self.model))


def import_sbml(text):
    """Import rate-rule ODEs with parameter assignment rules.

    Annotated QSPQuest documents preserve native units after verifying the SBML
    against the annotation. Unannotated documents use dimensionless numeric
    coordinates with explicit physical time, and report that limitation.
    """
    if not isinstance(text, str):
        _fail(
            "invalid_document",
            "import_sbml requires UTF-8 XML text, not a file path or bytes.",
        )
    lib = _libsbml()
    doc = _read_sbml(text)
    model = doc.getModel()
    for name, count in (
        ("reactions", model.getNumReactions()),
        ("events", model.getNumEvents()),
        ("initial assignments", model.getNumInitialAssignments()),
        ("constraints", model.getNumConstraints()),
        ("function definitions", model.getNumFunctionDefinitions()),
    ):
        if count:
            _fail(
                "unsupported",
                "SBML " + name + " are outside the rate-rule import subset.",
            )
    if any(not (r.isRate() or r.isAssignment()) for r in model.getListOfRules()):
        _fail("unsupported", "Algebraic rules are not supported.")
    root = _xml(text)
    annotation = root.find(".//{" + NS + "}nativeModel")
    digest = sha256(text.encode()).hexdigest()
    if annotation is not None:
        if annotation.get("profile") != SBML_PROFILE:
            _fail("unsupported", "Unknown SBML annotation profile.")
        try:
            native = json.loads(annotation.text)
            expected = export_sbml(native)
        except (ValueError, KeyError, TypeError) as exc:
            raise InterchangeError(
                "invalid_document", "Invalid native model annotation: " + str(exc)
            ) from exc
        if _signature(root) != _signature(_xml(expected)):
            _fail(
                "integrity",
                "SBML equations/values differ from the native model annotation.",
            )
        return ImportedModel(native, digest)
    if not model.getId() or not model.getNumSpecies():
        _fail("unsupported", "Import requires a named model with dynamic species.")
    # Restrict compartment semantics instead of converting concentrations into
    # amounts incorrectly. Species symbols remain their declared coordinates.
    for c in model.getListOfCompartments():
        if (
            not c.getConstant()
            or not c.isSetSize()
            or not math.isfinite(c.getSize())
            or c.getSize() <= 0
        ):
            _fail("unsupported", "Compartments must have fixed, finite positive sizes.")
    time_unit = "s" if model.getTimeUnits() == "second" else None
    definition = model.getUnitDefinition(model.getTimeUnits())
    if definition and definition.getNumUnits() == 1:
        unit = definition.getUnit(0)
        if unit.getKind() == lib.UNIT_KIND_SECOND and unit.getExponent() == 1:
            time_unit = {1.0: "s", 60.0: "min", 3600.0: "h", 86400.0: "day"}.get(
                unit.getMultiplier() * 10 ** unit.getScale()
            )
    if not time_unit:
        _fail(
            "unsupported",
            "Unannotated SBML requires explicit timeUnits resolving exactly to s, min, h or day.",
        )
    species = list(model.getListOfSpecies())
    parameters = list(model.getListOfParameters())
    names = {s.getId() for s in species} | {p.getId() for p in parameters}
    if names & {"pfx_coordinate_time", "time"}:
        _fail(
            "unsupported", "SBML symbol collides with a native coordinate identifier."
        )
    rates = {r.getVariable(): r for r in model.getListOfRules() if r.isRate()}
    assignments = {
        r.getVariable(): r for r in model.getListOfRules() if r.isAssignment()
    }
    if set(rates) != {s.getId() for s in species} or set(assignments) - {
        p.getId() for p in parameters
    }:
        _fail(
            "unsupported",
            "Each species requires one rate rule; assignment rules must target parameters.",
        )
    source = {
        "schema": "qspquest.model/v0.1",
        "model_id": model.getId(),
        "description": "Imported SBML numeric-coordinate ODE; original physical units are not inferred.",
        "time_unit": time_unit,
        "parameters": [],
        "states": [],
        "definitions": [],
        "outputs": [],
    }
    for p in parameters:
        if p.getId() in assignments:
            source["definitions"].append(
                {
                    "name": p.getId(),
                    "unit": "1",
                    "expression": _ast(assignments[p.getId()].getMath(), lib, names),
                }
            )
        elif p.getConstant() and p.isSetValue() and math.isfinite(p.getValue()):
            source["parameters"].append(
                {
                    "name": p.getId(),
                    "default": {"value": p.getValue(), "unit": "1"},
                    "bounds": None,
                    "transform": "identity",
                    "fixed": True,
                    "description": "SBML numeric coordinate",
                }
            )
        else:
            _fail("unsupported", "Non-assigned parameters must be finite constants.")
    source["parameters"].append(
        {
            "name": "pfx_coordinate_time",
            "default": {"value": 1, "unit": time_unit},
            "bounds": None,
            "transform": "identity",
            "fixed": True,
            "description": "One source time-coordinate unit",
        }
    )
    for s in species:
        if s.getConstant() or s.getBoundaryCondition() or s.isSetConversionFactor():
            _fail(
                "unsupported",
                "Constant/boundary species and conversion factors are unsupported.",
            )
        if s.getHasOnlySubstanceUnits() and s.isSetInitialAmount():
            initial = s.getInitialAmount()
        elif not s.getHasOnlySubstanceUnits() and s.isSetInitialConcentration():
            initial = s.getInitialConcentration()
        else:
            _fail(
                "unsupported",
                "Species must initialize the same amount/concentration coordinate used by its symbol.",
            )
        if not math.isfinite(initial):
            _fail("invalid_document", "Species initial values must be finite.")
        rhs = {
            "kind": "binary",
            "operator": "divide",
            "arguments": [
                _ast(rates[s.getId()].getMath(), lib, names),
                {"kind": "symbol", "name": "pfx_coordinate_time"},
            ],
        }
        source["states"].append(
            {
                "name": s.getId(),
                "unit": "1",
                "initial": {
                    "kind": "quantity",
                    "quantity": {"value": initial, "unit": "1"},
                },
                "rhs": rhs,
                "dosing": None,
            }
        )
        source["outputs"].append(
            {
                "name": "output_" + s.getId(),
                "unit": "1",
                "expression": {"kind": "symbol", "name": s.getId()},
            }
        )
    if model.isSetConversionFactor():
        _fail("unsupported", "Model conversion factors are unsupported.")
    result = ImportedModel(
        source,
        digest,
        (
            "Numeric-coordinate import: state, parameter and output units are dimensionless; no physical unit inference or dosing target inference.",
        ),
    )
    result.compile()
    return result


def _sedml(request):
    root = ET.Element("{" + SEDML_NS + "}sedML", {"level": "1", "version": "5"})
    annotation = ET.SubElement(root, "{" + SEDML_NS + "}annotation")
    experiment = ET.SubElement(
        annotation, "{" + NS + "}experiment", {"profile": PROFILE}
    )
    experiment.text = _json(request).decode()
    models = ET.SubElement(root, "{" + SEDML_NS + "}listOfModels")
    ET.SubElement(
        models,
        "{" + SEDML_NS + "}model",
        {"id": "model", "language": "urn:sedml:language:sbml", "source": "model.xml"},
    )
    # Exact nonuniform samples, pre/post boundaries and scheduled dosing live
    # in the explicit experiment annotation. No generic SED-ML runner is claimed.
    return ET.tostring(root, encoding="utf-8", xml_declaration=True)


def _check_request(model, request):
    from . import CompiledModel

    if not isinstance(request, dict) or request.get("schema") != "qspquest.run/v0.1":
        _fail("unsupported", "Archive scenarios require qspquest.run/v0.1 requests.")
    if request.get("solver") != "diffsol_bdf":
        _fail("unsupported", "Archive profile supports the diffsol_bdf solver only.")
    regimen = request.get("regimen", {})
    allowed = {"end", "samples", "observations", "administrations", "rtol", "atol"}
    if set(regimen) - allowed or set(request) - {
        "schema",
        "solver",
        "parameters",
        "regimen",
        "budgets",
    }:
        _fail(
            "unsupported",
            "Archive profile supports fixed bolus/infusion dosing, parameters and explicit observations only.",
        )
    for dose in regimen.get("administrations", []):
        if set(dose) - {
            "target",
            "amount",
            "time",
            "delivery",
            "bioavailability",
            "lag",
        } or dose.get("delivery", {}).get("kind") not in ("bolus", "infusion"):
            _fail(
                "unsupported",
                "Repeats, resets and additional administration semantics are outside this profile.",
            )
    # Native admission binds units, symbols, budgets and full event semantics;
    # compiling alone would not validate the scenario contract.
    from . import QSPQuestError

    try:
        CompiledModel(model).validate_run(request)
    except QSPQuestError as exc:
        raise InterchangeError(
            "invalid_document", "Native scenario admission failed: " + str(exc)
        ) from exc


def _check_fit(model, request):
    """Admit the narrow native statistical extension without fitting."""
    from . import CompiledSensitivities, QSPQuestError
    try:
        if request["schema"] != "qspquest.fit/v0.1" or request["problem"]["kind"] != "individual":
            _fail("unsupported", "Fit archives require native individual Gaussian fits.")
        objective = request["problem"]["request"]["objective"]
        if objective["priors"]:
            _fail("unsupported", "Fit archive profile requires empty priors.")
        simulation = objective["simulation"]
        run = simulation["run"]
        if run.get("solver") not in ("diffsol_bdf", "diffsol_rodas5p"):
            _fail("unsupported", "Fit archives require diffsol_bdf or diffsol_rodas5p.")
        # Check the fixed-regimen subset through native admission. This copy
        # changes only admission's solver, never the request retained in OMEX.
        admission = deepcopy(run)
        admission["solver"] = "diffsol_bdf"
        _check_request(model, admission)
        seen, errors = set(), {}
        rows = objective["observations"]
        if not isinstance(rows, list) or not 1 <= len(rows) <= 10_000:
            _fail("resource_limit", "Fit archive needs 1..10000 observation rows.")
        for row in rows:
            key = (row["row"], row["output"])
            if type(key[0]) is not int or not isinstance(key[1], str) or key in seen:
                _fail("invalid_document", "Observation (result row, output) keys must be unique and typed.")
            seen.add(key)
            error = row["error"]
            if row.get("censoring") is not None or error.get("model", "normal") != "normal" or type(error["proportional_sd"]) not in (int, float) or error["proportional_sd"] != 0:
                _fail("unsupported", "Fit archive requires uncensored normal errors with fixed additive SD and zero proportional SD.")
            sd = error["additive_sd"]
            if type(sd["value"]) not in (int, float) or not math.isfinite(sd["value"]) or sd["value"] <= 0:
                _fail("invalid_document", "Endpoint additive SD must be finite and positive.")
            signature = _json(error)
            if key[1] in errors and errors[key[1]] != signature:
                _fail("unsupported", "Each endpoint must declare the same fixed residual-error contract on every row.")
            errors[key[1]] = signature
        return CompiledSensitivities(model, simulation["with_respect_to"]).plan_fit(request)
    except QSPQuestError as exc:
        raise InterchangeError("invalid_document", "Native fit admission failed: " + str(exc)) from exc
    except (KeyError, TypeError, AttributeError, ValueError, OverflowError) as exc:
        if isinstance(exc, InterchangeError):
            raise
        raise InterchangeError("invalid_document", "Malformed native individual fit: " + str(exc)) from exc


@dataclass
class ImportedArchive:
    """Verified SBML model and named native scenarios from a bounded archive."""

    imported_model: ImportedModel
    scenarios: dict
    archive_sha256: str
    profile: str = PROFILE
    fits: dict = field(default_factory=dict)

    def run(self, scenario_id):
        """Execute one explicitly selected scenario through the native engine."""
        if scenario_id not in self.scenarios:
            _fail("invalid_document", "Unknown scenario: " + str(scenario_id))
        return self.imported_model.compile().run(deepcopy(self.scenarios[scenario_id]))

    def plan_fit(self, fit_id):
        """Return native admission/identity for a selected archived individual fit."""
        if fit_id not in self.fits:
            _fail("invalid_document", "Unknown fit: " + str(fit_id))
        return _check_fit(self.imported_model.model, deepcopy(self.fits[fit_id]))

    def fit(self, fit_id):
        """Execute an admitted fit; inspect the native fit.status for convergence."""
        from . import CompiledSensitivities
        self.plan_fit(fit_id)
        request = deepcopy(self.fits[fit_id])
        selection = request["problem"]["request"]["objective"]["simulation"]["with_respect_to"]
        return CompiledSensitivities(self.imported_model.model, selection).fit(request)


def _manifest(members):
    manifest = ET.Element("{" + MANIFEST_NS + "}omexManifest")
    ET.SubElement(
        manifest,
        "{" + MANIFEST_NS + "}content",
        {
            "location": ".",
            "format": "http://identifiers.org/combine.specifications/omex",
        },
    )
    for name in sorted(n for n in members if n != "manifest.xml"):
        format_name = (
            "sbml"
            if name.endswith(".xml")
            else "sed-ml" if name.endswith(".sedml") else None
        )
        attrs = {
            "location": "./" + name,
            "format": (
                "http://identifiers.org/combine.specifications/" + format_name
                if format_name
                else "application/json"
            ),
        }
        if name.endswith(".sedml"):
            attrs["master"] = "true"
        ET.SubElement(manifest, "{" + MANIFEST_NS + "}content", attrs)
    return ET.tostring(manifest, encoding="utf-8", xml_declaration=True)


def export_omex(model, scenarios, *, fits=None):
    """Return a deterministic COMBINE archive with SBML and experiment metadata.

    Accept a dict mapping scenario IDs to native run requests. The profile is
    intentionally explicit and is not a general SED-ML interchange contract.
    Optional named native individual ``fits`` select FIT_PROFILE. Fixed
    endpoint-specific Gaussian errors are a QSPQuest extension stored as JSON;
    no standard SED-ML or vendor fitting semantics are claimed.
    """
    source = model.model if isinstance(model, ImportedModel) else model
    fits = {} if fits is None else fits
    if not isinstance(scenarios, dict) or not isinstance(fits, dict) or not 1 <= len(scenarios) + len(fits) <= 100:
        _fail("resource_limit", "Provide between 1 and 100 named runs/fits in total.")
    if any(not isinstance(name, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,99}", name) for name in [*scenarios, *fits]):
        _fail("invalid_document", "Run and fit IDs must be short ASCII identifiers.")
    members = {"model.xml": export_sbml(source).encode()}
    entries = []
    for number, (name, request) in enumerate(sorted(scenarios.items())):
        if not isinstance(name, str) or not re.fullmatch(
            r"[A-Za-z_][A-Za-z0-9_-]{0,99}", name
        ):
            _fail("invalid_document", "Scenario IDs must be short ASCII identifiers.")
        _check_request(source, request)
        path = f"scenarios/{number}/experiment.sedml"
        # Each relative SED-ML model reference resolves to the matching SBML.
        members[f"scenarios/{number}/model.xml"] = members["model.xml"]
        members[path] = _sedml(request)
        entries.append(
            {
                "scenario_id": name,
                "sedml": path,
                "sbml": f"scenarios/{number}/model.xml",
            }
        )
    index = {
        "profile": PROFILE,
        "scenarios": entries,
        "members": {k: sha256(v).hexdigest() for k, v in members.items()},
    }
    if fits:
        fit_entries = []
        for number, (name, request) in enumerate(sorted(fits.items())):
            _check_fit(source, request)
            path = f"fits/{number}/request.json"
            members[path] = _json(request)
            fit_entries.append({"fit_id": name, "request": path})
        index.update(profile=FIT_PROFILE, fits=fit_entries,
                     members={k: sha256(v).hexdigest() for k, v in members.items()})
    members["index.json"] = _json(index)
    members["manifest.xml"] = _manifest(members)
    if sum(len(b) for b in members.values()) > MAX_EXPANDED_BYTES:
        _fail("resource_limit", "Export exceeds the 200 MB expanded archive limit.")
    buffer = BytesIO()
    with zipfile.ZipFile(buffer, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in sorted(members.items()):
            if len(data) > MAX_MEMBER_BYTES:
                _fail("resource_limit", "Archive member exceeds 10 MB.")
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = (stat.S_IFREG | 0o644) << 16
            archive.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED)
    data = buffer.getvalue()
    if len(data) > MAX_ARCHIVE_BYTES:
        _fail("resource_limit", "Archive exceeds 50 MB.")
    return data


def import_omex(data, *, expected_sha256=None):
    """Read bytes or a Path, verify inventory/hashes/profile and admit scenarios.

    An optional trusted SHA-256 authenticates the entire archive. Member hashes
    alone check consistency, not authorship. No extraction or network access.
    """
    if isinstance(data, (str, Path)):
        path = Path(data)
        if path.stat().st_size > MAX_ARCHIVE_BYTES:
            _fail("resource_limit", "Archive exceeds 50 MB.")
        data = path.read_bytes()
    if not isinstance(data, bytes) or not data or len(data) > MAX_ARCHIVE_BYTES:
        _fail("resource_limit", "Archive must contain between 1 byte and 50 MB.")
    digest = sha256(data).hexdigest()
    if (
        expected_sha256 is not None
        and expected_sha256.removeprefix("sha256:") != digest
    ):
        _fail("integrity", "Archive SHA-256 differs from the trusted digest.")
    try:
        with zipfile.ZipFile(BytesIO(data)) as archive:
            infos = archive.infolist()
            if (
                not infos
                or len(infos) > 5000
                or sum(i.file_size for i in infos) > MAX_EXPANDED_BYTES
            ):
                _fail(
                    "resource_limit", "Archive exceeds member or expanded-size limits."
                )
            members = {}
            for info in infos:
                # Validate the original name before ZipInfo normalizes it on Windows.
                name = info.orig_filename
                mode = info.external_attr >> 16
                if name != info.filename:
                    _fail("integrity", "Unsafe archive member name: " + name)
                if (
                    not name
                    or name in members
                    or "\\" in name
                    or name.startswith("/")
                    or any(p in (".", "..") for p in name.split("/"))
                    or ":" in name
                    or info.is_dir()
                    or stat.S_ISLNK(mode)
                    or (stat.S_IFMT(mode) not in (0, stat.S_IFREG))
                    or info.flag_bits & 1
                ):
                    _fail(
                        "integrity",
                        "Unsafe, duplicate or non-regular archive member: " + name,
                    )
                if info.file_size > MAX_MEMBER_BYTES:
                    _fail("resource_limit", "Archive member exceeds 10 MB.")
                members[name] = archive.read(info)
        index = _load_json(members["index.json"])
        profile = index.get("profile")
        if profile not in (PROFILE, FIT_PROFILE):
            _fail("unsupported", "Unsupported OMEX profile.")
        fields = {"profile", "scenarios", "members"} | ({"fits"} if profile == FIT_PROFILE else set())
        if set(index) != fields or set(
            index["members"]
        ) != set(members) - {"manifest.xml", "index.json"}:
            _fail("integrity", "Archive inventory differs from index.json.")
        if any(
            sha256(members[name]).hexdigest() != value
            for name, value in index["members"].items()
        ):
            _fail("integrity", "Archive member SHA-256 mismatch.")
        manifest = _xml(members["manifest.xml"])
        locations = [c.get("location") for c in manifest]
        if (
            manifest.tag != "{" + MANIFEST_NS + "}omexManifest"
            or len(set(locations)) != len(locations)
            or set(locations)
            != {"."} | {"./" + n for n in members if n != "manifest.xml"}
        ):
            _fail("integrity", "OMEX manifest differs from archive inventory.")
        if _signature(manifest) != _signature(_xml(_manifest(members))):
            _fail(
                "unsupported",
                "OMEX manifest format/master declarations differ from the profile.",
            )
        imported = import_sbml(members["model.xml"].decode())
        scenarios = {}
        used = {"model.xml"}
        fit_entries = index.get("fits", [])
        if not isinstance(index["scenarios"], list) or not isinstance(fit_entries, list) or not 1 <= len(index["scenarios"]) + len(fit_entries) <= 100 or (profile == FIT_PROFILE and not fit_entries):
            _fail("resource_limit", "Archive requires 1..100 runs/fits; fit profile requires at least one fit.")
        for entry in index["scenarios"]:
            name = entry["scenario_id"]
            if name in scenarios or set(entry) != {"scenario_id", "sedml", "sbml"}:
                _fail("integrity", "Scenario IDs or entries are not unique and exact.")
            tree = _xml(members[entry["sedml"]])
            experiment = tree.find(
                "{" + SEDML_NS + "}annotation/{" + NS + "}experiment"
            )
            if experiment is None or experiment.get("profile") != PROFILE:
                _fail("unsupported", "Scenario has no supported experiment annotation.")
            request = json.loads(experiment.text)
            if _signature(tree) != _signature(_xml(_sedml(request))):
                _fail("unsupported", "Changed or unsupported SED-ML semantics.")
            if (
                str(PurePosixPath(entry["sedml"]).parent / "model.xml") != entry["sbml"]
                or members[entry["sbml"]] != members["model.xml"]
            ):
                _fail("integrity", "Scenario model differs from the shared SBML model.")
            _check_request(imported.model, request)
            scenarios[name] = request
            used.update((entry["sedml"], entry["sbml"]))
        fits = {}
        for number, entry in enumerate(fit_entries):
            name = entry["fit_id"]
            if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,99}", name) or name in fits or set(entry) != {"fit_id", "request"}:
                _fail("integrity", "Fit IDs or entries are not unique and exact.")
            if entry["request"] != f"fits/{number}/request.json":
                _fail("integrity", "Fit request path differs from the canonical profile.")
            request = _load_json(members[entry["request"]])
            _check_fit(imported.model, request)
            fits[name] = request
            used.add(entry["request"])
        if used != set(index["members"]):
            _fail("unsupported", "Archive contains unused members outside the profile.")
        return ImportedArchive(imported, scenarios, digest, profile, fits)
    except InterchangeError:
        raise
    except (
        zipfile.BadZipFile,
        KeyError,
        ValueError,
        TypeError,
        UnicodeError,
        ET.ParseError,
        RuntimeError,
        AttributeError,
    ) as exc:
        raise InterchangeError(
            "invalid_document", "Malformed or inconsistent OMEX archive: " + str(exc)
        ) from exc
