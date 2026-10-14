"""Shared, non-executing structural translation into the native model schema."""

import ast
from copy import deepcopy
from dataclasses import dataclass, field
from hashlib import sha256
import math
import re
import sys


SOFTWARE = ("nonmem", "nlmixr2", "rxode2", "pumas", "winnonlin", "phoenix", "monolix")
MAX_SOURCE_BYTES = 1_000_000
MAX_SYMBOLS = 2000


class AdapterError(ValueError):
    """Invalid or unsupported source; no partial executable model is returned."""

    def __init__(self, message, *, code="unsupported", software=None, line=None):
        self.code, self.software, self.line = code, software, line
        super().__init__(message)


def fail(message, **kwargs):
    raise AdapterError(message, **kwargs)


def number(value, label="value"):
    if isinstance(value, bool):
        fail(label + " must be a finite number", code="invalid_input")
    try:
        value = float(value)
    except (TypeError, ValueError, OverflowError):
        fail(label + " must be a finite number", code="invalid_input")
    if not math.isfinite(value):
        fail(label + " must be finite", code="invalid_input")
    return value


def literal(value):
    return {"kind": "literal", "value": number(value)}


def symbol(name):
    return {"kind": "symbol", "name": name}


def binary(operator, left, right):
    return {"kind": "binary", "operator": operator, "arguments": [left, right]}


def split_top(text, separators=",\n;"):
    """Split only outside balanced parentheses/braces, preserving expression order."""
    result, start, stack, quote = [], 0, [], None
    for i, char in enumerate(text):
        if quote:
            if char == quote:
                quote = None
            continue
        if char in '"':
            quote = char
        elif char in "({[":
            stack.append(char)
        elif char in ")}]":
            if not stack or stack.pop() != {")": "(", "}": "{", "]": "["}[char]:
                fail("Unbalanced source delimiters", code="invalid_document")
        elif char in separators and not stack:
            part = text[start:i].strip()
            if part:
                result.append(part)
            start = i + 1
    if stack or quote:
        fail("Unbalanced source delimiters", code="invalid_document")
    if text[start:].strip():
        result.append(text[start:].strip())
    return result


def block(text, opening="{", closing="}"):
    """Consume a single enclosing block and reject trailing program statements."""
    text = text.strip()
    if not text.startswith(opening) or not text.endswith(closing):
        fail("Expected one enclosing model block", code="invalid_document")
    # A top-level split verifies that the closing delimiter really ends this block.
    split_top(text)
    return text[1:-1].strip()


def clean_source(source, software):
    if not isinstance(source, str) or not source.strip():
        fail("Source must be nonempty text, not a path", code="invalid_input")
    if len(source.encode("utf-8")) > MAX_SOURCE_BYTES:
        fail("Source exceeds the 1 MB adapter budget", code="resource_limit")
    if software not in SOFTWARE:
        fail("Unknown software: " + str(software), code="invalid_input")
    comment = ";" if software in ("nonmem", "monolix") else "#"
    return "\n".join(line.split(comment, 1)[0].strip() for line in source.splitlines())


@dataclass
class Program:
    parameters: dict = field(default_factory=dict)
    bounds: dict = field(default_factory=dict)
    definitions: dict = field(default_factory=dict)
    states: dict = field(default_factory=dict)
    initials: dict = field(default_factory=dict)
    outputs: dict = field(default_factory=dict)
    dosing: list = field(default_factory=list)
    metadata: list = field(default_factory=list)

    def assign(self, target, expression, section="definitions"):
        destination = getattr(self, section)
        if target in destination:
            fail("Repeated assignment requires procedural lowering: " + target)
        destination[target] = expression.strip()


_NAME = re.compile(r"[^\W\d]\w*(?:\.\w+)*", re.UNICODE)
_FUNCTIONS = {
    "exp": ("exp", 1),
    "log": ("log", 1),
    "ln": ("log", 1),
    "log10": ("log10", 1),
    "sqrt": ("sqrt", 1),
    "sin": ("sin", 1),
    "cos": ("cos", 1),
    "tan": ("tan", 1),
    "tanh": ("tanh", 1),
    "abs": ("abs", 1),
    "min": ("min", 2),
    "max": ("max", 2),
}
_OPERATORS = {
    ast.Add: "add",
    ast.Sub: "subtract",
    ast.Mult: "multiply",
    ast.Div: "divide",
    ast.Pow: "power",
}


def expression(text, mapping, time_unit, casefold=False):
    """Parse arithmetic only; never evaluate source code or arbitrary calls."""
    if len(text) > 10000:
        fail("Expression exceeds 10000 characters", code="resource_limit")
    text = re.sub(r"(?<=\d)[dD]([+-]?\d+)", r"e\1", text).replace("^", "**")
    text = text.replace("@.", "")
    # Indexed NONMEM/WinNonlin/Pumas quantities have already been named by the reader.
    resolved = {}

    def name(match):
        original = match.group()
        key = original.upper() if casefold else original
        if key in mapping:
            token = "pfx_symbol_" + str(len(resolved))
            resolved[token] = symbol(mapping[key])
            return token
        if key in (("T", "TIME", "X") if casefold else ("t", "time")):
            token = "pfx_symbol_" + str(len(resolved))
            resolved[token] = binary(
                "divide", symbol("time"), symbol("pfx_coordinate_time")
            )
            return token
        if original.lower() in _FUNCTIONS:
            return original.lower()
        fail("Unbound source symbol or unsupported function: " + original)

    # Keep scientific-notation exponent letters out of identifier substitution.
    text = re.sub(r"(?<![\w.])[^\W\d]\w*(?:\.\w+)*", name, text)
    try:
        tree = ast.parse(text.strip(), mode="eval")
    except (SyntaxError, ValueError, RecursionError) as error:
        fail(
            "Unsupported arithmetic expression: " + str(error), code="invalid_document"
        )
    if sum(1 for _ in ast.walk(tree)) > 1000:
        fail("Expression exceeds 1000 nodes", code="resource_limit")

    def lower(node, depth=0):
        if depth > 32:
            fail("Expression depth exceeds 32", code="resource_limit")
        if isinstance(node, ast.Constant) and isinstance(node.value, (float, int)):
            return literal(node.value)
        if isinstance(node, ast.Name) and node.id in resolved:
            return deepcopy(resolved[node.id])
        if isinstance(node, ast.BinOp) and type(node.op) in _OPERATORS:
            return binary(
                _OPERATORS[type(node.op)],
                lower(node.left, depth + 1),
                lower(node.right, depth + 1),
            )
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.UAdd, ast.USub)):
            value = lower(node.operand, depth + 1)
            return (
                value
                if isinstance(node.op, ast.UAdd)
                else binary("subtract", literal(0), value)
            )
        if (
            isinstance(node, ast.Call)
            and isinstance(node.func, ast.Name)
            and node.func.id in _FUNCTIONS
        ):
            target, arity = _FUNCTIONS[node.func.id]
            if node.keywords or len(node.args) != arity:
                fail("Unsupported function arguments: " + node.func.id)
            return {
                "kind": "call",
                "name": target,
                "arguments": [lower(a, depth + 1) for a in node.args],
            }
        fail("Unsupported expression construct: " + type(node).__name__)

    return lower(tree.body)


@dataclass
class AdaptedModel:
    """A native executable plus source provenance and explicit translation boundaries."""

    model: dict
    software: str
    source_sha256: str
    symbol_map: dict
    compartment_map: dict
    output_map: dict
    source_metadata: tuple
    warnings: tuple
    _program: Program = field(repr=False)

    def compile(self):
        from .. import CompiledModel

        return CompiledModel(deepcopy(self.model))

    def to_dict(self):
        return deepcopy(
            {
                "schema": "qspquest.software-adapter/v1",
                "software": self.software,
                "model": self.model,
                "source_sha256": self.source_sha256,
                "symbol_map": self.symbol_map,
                "compartment_map": self.compartment_map,
                "output_map": self.output_map,
                "source_metadata": self.source_metadata,
                "warnings": self.warnings,
            }
        )


def native_model(
    program, software, source, model_id, time_unit, overrides, outputs, dosing_targets
):
    time_unit = "d" if time_unit == "day" else time_unit
    if time_unit not in ("s", "min", "h", "d"):
        fail("time_unit must explicitly be s, min, h or d", code="invalid_input")
    if not re.fullmatch(r"[a-z_][a-z0-9_]{0,127}", model_id or ""):
        fail("model_id must be a native identifier", code="invalid_input")
    folded = software in ("nonmem", "winnonlin")
    if outputs is not None and (not isinstance(outputs, dict) or not outputs):
        fail(
            "outputs must map source readout names to expressions", code="invalid_input"
        )
    source_symbols = set()
    for value in [
        *program.definitions.values(),
        *program.states.values(),
        *program.initials.values(),
        *program.outputs.values(),
        *(outputs or {}).values(),
    ]:
        if not isinstance(value, str):
            fail("Source expressions must be text", code="invalid_input")
        source_symbols.update(n.upper() if folded else n for n in _NAME.findall(value))
    overrides = dict(overrides or {})
    for name, value in overrides.items():
        if not isinstance(name, str) or not _NAME.fullmatch(name):
            fail("Parameter binding must name a source symbol", code="invalid_input")
        key = name.upper() if folded else name
        if key in program.states or key in program.definitions:
            fail("A parameter binding cannot replace a state or assignment: " + name)
        if key not in program.parameters and key not in source_symbols:
            fail(
                "Parameter binding names no source symbol: " + name,
                code="invalid_input",
            )
        program.parameters[key] = number(value, name)
    if outputs is not None:
        if not isinstance(outputs, dict) or not outputs:
            fail(
                "outputs must map source readout names to expressions",
                code="invalid_input",
            )
        program.outputs = dict(outputs)
    if not program.outputs:
        program.outputs = {"state_" + n: n for n in program.states}
    if dosing_targets is not None:
        program.dosing = list(dosing_targets)
    if not program.states or not program.outputs:
        fail("Import requires ODE states and deterministic outputs")
    if set(program.initials) - set(program.states) or set(program.dosing) - set(
        program.states
    ):
        fail("Initial conditions and dosing targets must name declared ODE states")
    if len(set(program.dosing)) != len(program.dosing):
        fail("Duplicate dosing targets", code="invalid_input")
    names = [*program.parameters, *program.definitions, *program.states]
    if len(names) > MAX_SYMBOLS:
        fail("Model exceeds the 2000 symbol budget", code="resource_limit")
    if len(names) != len(set(names)):
        fail("A symbol is declared in more than one source section")
    mapping = {name: "src_" + str(i) for i, name in enumerate(names)}
    parse = lambda value: expression(value, mapping, time_unit, folded)
    model = {
        "schema": "qspquest.model/v0.1",
        "model_id": model_id,
        "description": "Imported " + software + " deterministic numeric-coordinate ODE",
        "time_unit": time_unit,
        "parameters": [],
        "definitions": [],
        "states": [],
        "outputs": [],
    }
    for name, value in program.parameters.items():
        if value is None:
            fail("Explicit individual parameter value required: " + name)
        bounds = program.bounds.get(name)
        if bounds is not None:
            lower, upper = bounds
            if lower is not None and upper is not None and lower > upper:
                fail("Invalid source bounds: " + name)
            if (
                lower is not None
                and value < lower
                or upper is not None
                and value > upper
            ):
                fail("Individual parameter violates source bounds: " + name)
            bounds = [
                {
                    "value": lower if lower is not None else -sys.float_info.max,
                    "unit": "1",
                },
                {
                    "value": upper if upper is not None else sys.float_info.max,
                    "unit": "1",
                },
            ]
        model["parameters"].append(
            {
                "name": mapping[name],
                "default": {"value": number(value), "unit": "1"},
                "bounds": bounds,
                "transform": "identity",
                "fixed": True,
                "description": "Source scalar coordinate",
            }
        )
    model["parameters"].append(
        {
            "name": "pfx_coordinate_time",
            "default": {"value": 1, "unit": time_unit},
            "bounds": [{"value": 1, "unit": time_unit}] * 2,
            "transform": "identity",
            "fixed": True,
            "description": "One source time-coordinate unit",
        }
    )
    model["definitions"] = [
        {"name": mapping[n], "unit": "1", "expression": parse(v)}
        for n, v in program.definitions.items()
    ]
    for name, rhs in program.states.items():
        initial = parse(program.initials.get(name, "0"))
        derivative = binary("divide", parse(rhs), symbol("pfx_coordinate_time"))
        dosing = None
        if name in program.dosing:
            derivative = binary("add", derivative, symbol("input_" + mapping[name]))
            dosing = {"amount_unit": "1", "scale": literal(1)}
        model["states"].append(
            {
                "name": mapping[name],
                "unit": "1",
                "initial": {"kind": "expression", "expression": initial},
                "rhs": derivative,
                "dosing": dosing,
            }
        )
    output_map = {}
    for i, (name, value) in enumerate(program.outputs.items()):
        output_map[name] = "readout_" + str(i)
        model["outputs"].append(
            {"name": output_map[name], "unit": "1", "expression": parse(value)}
        )
    warnings = [
        "Numeric-coordinate import: parameters, states, doses and outputs use unit 1; physical units are not inferred.",
        "Deterministic individual simulation only; population estimation and stochastic observation models are not translated.",
    ]
    if program.metadata:
        warnings.append(
            "Source metadata is retained for review and is not executed; supply datasets and individual effects explicitly."
        )
    result = AdaptedModel(
        model,
        software,
        sha256(source.encode()).hexdigest(),
        mapping,
        {i + 1: mapping[n] for i, n in enumerate(program.states)},
        output_map,
        tuple(program.metadata),
        tuple(warnings),
        deepcopy(program),
    )
    result.compile()
    return result
