"""Deterministic text exports of models admitted by the structural adapters."""

from dataclasses import dataclass

from ._core import AdaptedModel, expression, fail


@dataclass
class ExportedModel:
    """Text and explicit bindings required to reimport the individual model.

    This is structural source, not a complete fitting project or vendor dataset.
    """

    software: str
    source: str
    parameters: dict
    outputs: dict
    dosing_targets: tuple
    time_unit: str
    symbol_map: dict

    def import_back(self, *, model_id="adapter_roundtrip"):
        from . import import_model

        return import_model(
            self.source,
            self.software,
            model_id=model_id,
            time_unit=self.time_unit,
            parameters=self.parameters,
            outputs=self.outputs,
            dosing_targets=self.dosing_targets,
        )


def export_model(imported, software):
    """Export the deterministic numeric-coordinate subset to another source DSL.

    Only AdaptedModel objects are accepted. Arbitrary native physical-unit models
    need SBML/OMEX export; this function does not infer vendor unit conventions.
    """
    from ._core import SOFTWARE

    if not isinstance(imported, AdaptedModel) or software not in SOFTWARE:
        fail(
            "export_model requires an AdaptedModel and a supported software name",
            code="invalid_input",
        )
    p = imported._program
    if p.bounds and software not in ("pumas", "nonmem", "phoenix"):
        fail(
            "This destination subset cannot retain parameter bounds; use SBML/OMEX or a supported bounds destination"
        )
    if software in ("nonmem", "winnonlin") and len(p.outputs) != 1:
        fail("This NONMEM/WinNonlin export subset supports one observation readout")
    names = {}
    for i, name in enumerate(p.parameters, 1):
        names[name] = (
            "THETA(" + str(i) + ")"
            if software == "nonmem"
            else "P(" + str(i) + ")" if software == "winnonlin" else "par" + str(i)
        )
    for i, name in enumerate(p.definitions, 1):
        names[name] = (
            "DEF" + str(i) if software in ("nonmem", "winnonlin") else "def" + str(i)
        )
    for i, name in enumerate(p.states, 1):
        names[name] = (
            "A(" + str(i) + ")"
            if software == "nonmem"
            else "Z(" + str(i) + ")" if software == "winnonlin" else "state" + str(i)
        )
    inverse = {imported.symbol_map[name]: value for name, value in names.items()}
    folded = imported.software in ("nonmem", "winnonlin")

    def render(node):
        kind = node["kind"]
        if kind == "literal":
            return repr(float(node["value"]))
        if kind == "symbol":
            return inverse[node["name"]]
        args = node.get("arguments", [])
        if kind == "binary":
            if (
                node["operator"] == "divide"
                and args[0] == {"kind": "symbol", "name": "time"}
                and args[1] == {"kind": "symbol", "name": "pfx_coordinate_time"}
            ):
                return (
                    "X"
                    if software == "winnonlin"
                    else "T" if software == "nonmem" else "t"
                )
            op = {
                "add": "+",
                "subtract": "-",
                "multiply": "*",
                "divide": "/",
                "power": "^",
            }[node["operator"]]
            if software in ("nonmem", "winnonlin") and op == "^":
                op = "**"
            return "(" + render(args[0]) + op + render(args[1]) + ")"
        if kind == "call":
            name = (
                node["name"].upper()
                if software in ("nonmem", "winnonlin")
                else node["name"]
            )
            return name + "(" + ",".join(render(a) for a in args) + ")"
        fail("Unsupported export expression")

    def rhs(value):
        return render(
            expression(value, imported.symbol_map, imported.model["time_unit"], folded)
        )

    parameters = {names[n]: v for n, v in p.parameters.items()}

    def bound_values(name):
        lower, upper = p.bounds.get(name, (None, None))
        return (
            "" if lower is None else repr(lower),
            "" if upper is None else repr(upper),
        )

    # Export dependencies before their uses, including state-dependent aliases.
    ordered, pending = [], dict(p.definitions)
    dynamic = set(imported.symbol_map[n] for n in p.states) | {"time"}

    def dependencies(node):
        return (
            {node["name"]}
            if node["kind"] == "symbol"
            else set().union(*(dependencies(a) for a in node.get("arguments", [])))
        )

    while pending:
        ready = [
            n
            for n, v in pending.items()
            if not (
                dependencies(
                    expression(
                        v, imported.symbol_map, imported.model["time_unit"], folded
                    )
                )
                & {imported.symbol_map[k] for k in pending}
            )
        ]
        if not ready:
            fail("Cyclic export definitions")
        for name in ready:
            value = pending.pop(name)
            if (
                dependencies(
                    expression(
                        value, imported.symbol_map, imported.model["time_unit"], folded
                    )
                )
                & dynamic
            ):
                dynamic.add(imported.symbol_map[name])
            ordered.append(name)
    definitions = [names[n] + " = " + rhs(p.definitions[n]) for n in ordered]
    static_definitions = [
        names[n] + " = " + rhs(p.definitions[n])
        for n in ordered
        if imported.symbol_map[n] not in dynamic
    ]
    dynamic_definitions = [
        names[n] + " = " + rhs(p.definitions[n])
        for n in ordered
        if imported.symbol_map[n] in dynamic
    ]
    readouts = {"output" + str(i): rhs(v) for i, v in enumerate(p.outputs.values(), 1)}
    defaults = {names[n]: rhs(p.initials.get(n, "0")) for n in p.states}
    bindings, explicit_outputs = {}, {n: n for n in readouts}
    dosing = tuple(names[n] for n in p.dosing)
    if software == "nonmem":
        if any(p.initials.get(n, "0").strip() not in ("0", "0.0") for n in p.states):
            fail("NONMEM nonzero initial-state export needs A_0 lowering")
        expanded_nodes = 0

        def expanded(node, depth=0):
            nonlocal expanded_nodes
            expanded_nodes += 1
            if depth > 32 or expanded_nodes > 10000:
                fail(
                    "Expanded NONMEM readout exceeds the export budget",
                    code="resource_limit",
                )
            if node["kind"] == "symbol":
                name = next(
                    (
                        n
                        for n in p.definitions
                        if imported.symbol_map[n] == node["name"]
                    ),
                    None,
                )
                if name is not None:
                    return expanded(
                        expression(
                            p.definitions[name],
                            imported.symbol_map,
                            imported.model["time_unit"],
                            folded,
                        ),
                        depth + 1,
                    )
            result = dict(node)
            if "arguments" in node:
                result["arguments"] = [
                    expanded(a, depth + 1) for a in node["arguments"]
                ]
            return result

        output = render(
            expanded(
                expression(
                    next(iter(p.outputs.values())),
                    imported.symbol_map,
                    imported.model["time_unit"],
                    folded,
                )
            )
        )
        theta = []
        for name, value in p.parameters.items():
            if name in p.bounds:
                lower, upper = bound_values(name)
                theta.append(
                    "("
                    + (lower or "-1.7976931348623157e308")
                    + ","
                    + repr(value)
                    + ","
                    + upper
                    + ") FIX"
                )
            else:
                theta.append(repr(value) + " FIX")
        lines = [
            "$PROBLEM QSPQuest deterministic structural export",
            "$SUBROUTINES ADVAN13 TOL=9",
            "$MODEL "
            + " ".join(
                "COMP=(STATE" + str(i) + ")" for i in range(1, len(p.states) + 1)
            ),
            "$THETA " + " ".join(theta),
            "$PK",
            *static_definitions,
            "$DES",
            *dynamic_definitions,
            *[
                "DADT(" + str(i) + ") = " + rhs(v)
                for i, v in enumerate(p.states.values(), 1)
            ],
            "$ERROR",
            "Y = " + output,
        ]
        dosing = tuple(
            "STATE" + str(i) for i, n in enumerate(p.states, 1) if n in p.dosing
        )
        explicit_outputs = {"output1": "Y"}
    elif software == "winnonlin":
        lines = [
            "COMMANDS",
            "NPARAMETERS " + str(len(parameters)),
            "INITIAL " + ", ".join(repr(v) for v in parameters.values()),
            "NDERIVATIVES " + str(len(p.states)),
            "NFUNCTIONS 1",
            "END",
            "TEMPORARY",
            *definitions,
            "END",
            "STARTING",
            *[
                "Z(" + str(i) + ") = " + defaults[names[n]]
                for i, n in enumerate(p.states, 1)
            ],
            "END",
            "DIFFERENTIAL",
            *[
                "DZ(" + str(i) + ") = " + rhs(v)
                for i, v in enumerate(p.states.values(), 1)
            ],
            "END",
            "FUNCTION 1",
            "F = " + next(iter(readouts.values())),
            "END",
        ]
        dosing = tuple(
            "Z_" + str(i) for i, n in enumerate(p.states, 1) if n in p.dosing
        )
        explicit_outputs = {"output1": "F"}
    elif software in ("nlmixr2", "rxode2"):
        body = [
            *definitions,
            *[names[n] + "(0) = " + defaults[names[n]] for n in p.states],
            *["d/dt(" + names[n] + ") = " + rhs(v) for n, v in p.states.items()],
            *[n + " = " + v for n, v in readouts.items()],
        ]
        if software == "nlmixr2":
            lines = [
                "function() {",
                "ini({",
                *[n + " = " + repr(v) for n, v in parameters.items()],
                "})",
                "model({",
                *body,
                "})",
                "}",
            ]
        else:
            lines = ["{", *body, "}"]
            bindings = parameters
    elif software == "pumas":
        declarations = []
        for name, value in p.parameters.items():
            lower, upper = bound_values(name)
            fields = ["init=" + repr(value)]
            if lower:
                fields.append("lower=" + lower)
            if upper:
                fields.append("upper=" + upper)
            declarations.append(
                names[name] + " ∈ RealDomain(" + ", ".join(fields) + ")"
            )
        lines = [
            "@model begin",
            "@param begin",
            *declarations,
            "end",
            "@pre begin",
            *static_definitions,
            "end",
            "@vars begin",
            *dynamic_definitions,
            "end",
            "@init begin",
            *[n + " = " + v for n, v in defaults.items()],
            "end",
            "@dynamics begin",
            *[names[n] + "' = " + rhs(v) for n, v in p.states.items()],
            "end",
            "@observed begin",
            *[n + " = " + v for n, v in readouts.items()],
            "end",
            "end",
        ]
    elif software == "phoenix":
        declarations = [
            "fixef("
            + names[n]
            + " = c("
            + bound_values(n)[0]
            + ","
            + repr(v)
            + ","
            + bound_values(n)[1]
            + "))"
            for n, v in p.parameters.items()
        ]
        lines = [
            "qspquest_export(){",
            *declarations,
            *definitions,
            *["deriv(" + names[n] + " = " + rhs(v) + ")" for n, v in p.states.items()],
            *["dosepoint(" + n + ")" for n in dosing],
            *[n + " = " + v for n, v in readouts.items()],
            "}",
        ]
        if any(p.initials.get(n, "0").strip() not in ("0", "0.0") for n in p.states):
            fail("Nonzero PML initial states are outside this export subset")
    else:
        lines = [
            "[LONGITUDINAL]",
            "input = {" + ", ".join(parameters) + "}",
            "PK:",
            *["depot(target=" + n + ")" for n in dosing],
            "EQUATION:",
            "t_0 = 0",
            *definitions,
            *[n + "_0 = " + v for n, v in defaults.items()],
            *["ddt_" + names[n] + " = " + rhs(v) for n, v in p.states.items()],
            *[n + " = " + v for n, v in readouts.items()],
            "OUTPUT:",
            "output = {" + ", ".join(readouts) + "}",
        ]
        bindings = parameters
    return ExportedModel(
        software,
        "\n".join(lines) + "\n",
        bindings,
        explicit_outputs,
        dosing,
        imported.model["time_unit"],
        names,
    )
