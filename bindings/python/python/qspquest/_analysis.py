"""Argument conversion and validation helpers for native analyses."""

import json
import math

from . import _native


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


def _plain(value):
    # Arrays and scalars from NumPy, pandas or similar libraries are accepted
    # by duck typing; QSPQuest itself imports none of them.
    if hasattr(value, "tolist"):
        return value.tolist()
    if hasattr(value, "item"):
        return value.item()
    raise TypeError(
        "analysis inputs must be JSON-compatible quantities, numbers and arrays"
    )


def encode(arguments):
    try:
        return json.dumps(
            arguments, allow_nan=False, separators=(",", ":"), default=_plain
        )
    except (TypeError, ValueError) as error:
        fail(str(error))


def named(mapping, name):
    """Convert an ordered name-to-specification mapping to a named list."""
    if not isinstance(mapping, dict):
        fail(f"{name} must be a mapping of names to specifications")
    items = []
    for key, value in mapping.items():
        if not isinstance(key, str) or not isinstance(value, dict):
            fail(f"{name} names and specifications are required")
        if "name" in value:
            fail(f"{name} specifications must not repeat the name field")
        items.append({"name": key, **value})
    return items


def in_declared_order(result, *orders):
    """Return ``result`` with every mapping keyed by a declared name set in that order.

    Native records serialize name-keyed mappings in sorted key order. Each of
    ``orders`` is a sequence of names, such as fitted parameters or regimen
    labels; any mapping, at any depth, whose keys are exactly one of those
    sets is rebuilt in the declared order.
    """
    orders = [list(names) for names in orders if names]
    sets = [set(names) for names in orders]

    def visit(value):
        if isinstance(value, dict):
            for item in value.values():
                visit(item)
            keys = set(value)
            for names, key_set in zip(orders, sets):
                if keys == key_set and list(value) != names:
                    items = {name: value.pop(name) for name in names}
                    value.update(items)
                    break
        elif isinstance(value, list):
            for item in value:
                visit(item)

    visit(result)
    return result


def native(model, operation, arguments, control=None):
    """Run one native analysis and return its result record."""
    from . import QSPQuestError

    arguments = {k: v for k, v in arguments.items() if v is not None}
    try:
        if model is None:
            text = _native.analysis_record_json(operation, encode(arguments))
        else:
            inner = getattr(model, "_model", None)
            if inner is None:
                fail("analysis requires a qspquest.CompiledModel")
            text = inner.analysis_json(operation, encode(arguments), control)
    except _native.NativeError as error:
        raise QSPQuestError(json.loads(str(error))) from None
    return json.loads(text)
