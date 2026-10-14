"""Explicit text subsets of seven pharmacometric model languages."""

import re

from ._core import Program, block, fail, number, split_top


NAME = r"[^\W\d]\w*(?:\.\w+)*"
ASSIGNMENT = re.compile(r"(" + NAME + r")\s*(?:<-|:=|=(?!=))\s*(.+)", re.S)


def assignment(statement, program, section="definitions"):
    match = ASSIGNMENT.fullmatch(statement.strip())
    if not match:
        fail("Unsupported statement: " + statement[:120])
    program.assign(*match.groups(), section)


def statistical(statement, program, structural_only):
    if not structural_only:
        fail(
            "Statistical/estimation declaration requires structural_only=True: "
            + statement[:100]
        )
    program.metadata.append(statement)


def _indexed(text, aliases, allowed):
    def replace(match):
        kind, index = match.group(1).upper(), int(match.group(2))
        if kind not in {"A", "THETA", "ETA", "EPS", "Z", "P", "CON", "DADT", "DZ"}:
            return match.group()
        if (kind, index) not in aliases:
            if kind not in allowed or index < 1:
                fail("Unsupported or undeclared indexed variable: " + match.group())
            aliases[kind, index] = kind.lower() + "_" + str(index)
        return aliases[kind, index]

    return re.sub(r"\b([A-Za-z]+)\s*\(\s*(\d+)\s*\)", replace, text)


def nonmem(text, *, structural_only, **_):
    program, sections, current = Program(), {}, None
    for line in text.splitlines():
        if not line:
            continue
        if line.startswith("$"):
            match = re.fullmatch(r"\$([A-Za-z]+)\s*(.*)", line)
            if not match:
                fail("Invalid NONMEM record")
            current, rest = match.groups()
            current = {"SUBR": "SUBROUTINES", "SUBROUTINE": "SUBROUTINES"}.get(
                current.upper(), current.upper()
            )
            if current in sections:
                fail("Repeated NONMEM record is outside this subset: " + current)
            sections[current] = [rest]
        elif current:
            sections[current].append(line)
        else:
            fail("NONMEM content must start with a record")
    allowed = {
        "PROBLEM",
        "INPUT",
        "DATA",
        "MODEL",
        "SUBROUTINES",
        "THETA",
        "OMEGA",
        "SIGMA",
        "PK",
        "DES",
        "ERROR",
        "ESTIMATION",
        "COVARIANCE",
        "TABLE",
    }
    if set(sections) - allowed:
        fail("Unsupported NONMEM record: " + ", ".join(sorted(set(sections) - allowed)))
    for key in ("PROBLEM", "INPUT", "DATA", "SUBROUTINES", "TABLE"):
        if key in sections:
            program.metadata.append("$" + key + " " + " ".join(sections[key]))
    for key in ("OMEGA", "SIGMA", "ESTIMATION", "COVARIANCE"):
        if key in sections:
            statistical(
                "$" + key + " " + " ".join(sections[key]), program, structural_only
            )
    subroutine = " ".join(sections.get("SUBROUTINES", []))
    if not re.fullmatch(r"ADVAN(?:6|8|13)(?:\s+TOL\s*=\s*\d+)?", subroutine, re.I):
        fail(
            "Direct NONMEM import supports only explicit ADVAN6/8/13 $DES; use the optional converter bridge for other ADVANs"
        )
    model = " ".join(sections.get("MODEL", []))
    declarations = re.findall(r"COMP(?:ARTMENT)?\s*=\s*\(([^)]+)\)", model, re.I)
    remainder = re.sub(r"COMP(?:ARTMENT)?\s*=\s*\([^)]+\)", "", model, flags=re.I)
    if not declarations or remainder.strip():
        fail("$MODEL requires named COMP=(name[,DEFDOS,DEFOBS]) declarations")
    aliases, observation = {}, None
    for i, declaration in enumerate(declarations, 1):
        fields = [s.strip().strip("\"'").upper() for s in declaration.split(",")]
        if not re.fullmatch(NAME, fields[0]) or set(fields[1:]) - {"DEFDOS", "DEFOBS"}:
            fail("Unsupported NONMEM compartment declaration: " + declaration)
        aliases["A", i] = fields[0]
        if "DEFOBS" in fields[1:]:
            if observation is not None:
                fail("Multiple default observation compartments")
            observation = i
    theta_text = " ".join(sections.get("THETA", []))
    theta_tokens = re.findall(
        r"\([^()]+\)(?:\s*FIX(?:ED)?)?|[-+]?\d*\.?\d+(?:[eEdD][-+]?\d+)?(?:\s*FIX(?:ED)?)?",
        theta_text,
        re.I,
    )
    residual = theta_text
    for token in theta_tokens:
        residual = residual.replace(token, "", 1)
    if residual.strip() or not theta_tokens:
        fail("Unsupported $THETA syntax")
    for i, token in enumerate(theta_tokens, 1):
        token = re.sub(r"\s*FIX(?:ED)?$", "", token, flags=re.I).strip()
        values = token.strip("()").split(",")
        if len(values) not in (1, 2, 3) or not values[0].strip():
            fail("$THETA requires a numeric initial estimate")
        value = values[0] if len(values) == 1 else values[1]
        for bound in values:
            if bound.strip():
                number(re.sub(r"[dD]", "e", bound))
        aliases["THETA", i] = "THETA_" + str(i)
        program.parameters[aliases["THETA", i]] = number(re.sub(r"[dD]", "e", value))
        if len(values) > 1:
            program.bounds[aliases["THETA", i]] = (
                number(re.sub(r"[dD]", "e", values[0])),
                (
                    number(re.sub(r"[dD]", "e", values[2]))
                    if len(values) == 3 and values[2].strip()
                    else None
                ),
            )
    program.metadata.append("$THETA " + theta_text)
    for key, section in (
        ("PK", "definitions"),
        ("DES", "states"),
        ("ERROR", "definitions"),
    ):
        for line in sections.get(key, []):
            if not line:
                continue
            if key == "DES":
                match = re.fullmatch(r"DADT\s*\(\s*(\d+)\s*\)\s*=\s*(.+)", line, re.I)
                if not match:
                    assignment(_indexed(line, aliases, {"ETA"}).upper(), program)
                    continue
                if ("A", int(match[1])) not in aliases:
                    fail("$DES requires a declared DADT(n) assignment")
                program.assign(
                    aliases["A", int(match[1])],
                    _indexed(match[2], aliases, {"ETA"}).upper(),
                    section,
                )
            else:
                expression = _indexed(line, aliases, {"ETA"}).upper()
                assignment(expression, program)
    if set(program.states) != {
        aliases["A", i] for i in range(1, len(declarations) + 1)
    }:
        fail("Every $MODEL compartment requires a $DES equation")
    # Require source order to match CMT even if $DES equations are reordered.
    program.states = {
        aliases["A", i]: program.states[aliases["A", i]]
        for i in range(1, len(declarations) + 1)
    }
    for name in program.definitions:
        if re.fullmatch(r"(?:F|ALAG|D|R)\d+", name):
            fail("NONMEM modeled dosing attributes need explicit lowering: " + name)
    if any(re.search(r"\bF\b", rhs) for rhs in program.definitions.values()):
        if observation is None:
            fail("NONMEM F readout requires an explicit DEFOBS compartment")
        i = observation
        scale = "S" + str(i)
        program.assign(
            "F", aliases["A", i] + ("/" + scale if scale in program.definitions else "")
        )
    if "Y" in program.definitions:
        program.outputs = {"Y": "Y"}
    elif "IPRED" in program.definitions:
        program.outputs = {"IPRED": "IPRED"}
    else:
        fail(
            "$ERROR must define deterministic Y or IPRED; EPS-based residuals are unsupported by the direct importer"
        )
    program.dosing = list(program.states)
    return program


def _r_statements(text, program, *, ini=False, structural_only=False):
    for statement in split_top(text, separators="\n;"):
        if "~" in statement:
            statistical(statement, program, structural_only)
            continue
        derivative = re.fullmatch(
            r"d/dt\(\s*(" + NAME + r")\s*\)\s*(?:<-|=)\s*(.+)", statement, re.S
        )
        initial = re.fullmatch(
            r"(" + NAME + r")\(0\)\s*(?:<-|=)\s*(.+)", statement, re.S
        )
        if derivative and not ini:
            program.assign(*derivative.groups(), "states")
        elif initial and not ini:
            program.assign(*initial.groups(), "initials")
        elif ini:
            match = ASSIGNMENT.fullmatch(statement)
            if not match:
                fail("Unsupported ini declaration: " + statement)
            name, value = match.groups()
            # Bounds vectors and arbitrary R calls require the external AST bridge.
            if name in program.parameters:
                fail("Repeated ini parameter declaration: " + name)
            program.parameters[name] = number(value, name)
        else:
            assignment(statement, program)


def rxode2(text, *, structural_only=False, **_):
    program = Program()
    wrapper = re.match(r"(?:rxode2::)?rxode2\s*\(\s*", text)
    if wrapper:
        if not text.rstrip().endswith(")"):
            fail("Unclosed rxode2 constructor")
        text = text[wrapper.end() :].strip()[:-1].strip()
    if text.startswith("{"):
        text = block(text)
    _r_statements(text, program, structural_only=structural_only)
    program.dosing = list(program.states)
    return program


def nlmixr2(text, *, structural_only=False, **_):
    program = Program()
    match = re.match(r"(?:(?:" + NAME + r")\s*(?:<-|=)\s*)?function\s*\(\s*\)\s*", text)
    if not match:
        fail("nlmixr2 import expects one function() with ini({}) and model({})")
    body = block(text[match.end() :])
    parts = split_top(body, separators="\n;")
    seen = set()
    for part in parts:
        match = re.fullmatch(r"(ini|model)\s*\(\s*(\{.*\})\s*\)", part, re.S)
        if not match or match[1] in seen:
            fail("Only one ini({}) and one model({}) block are supported")
        seen.add(match[1])
        _r_statements(
            block(match[2]),
            program,
            ini=match[1] == "ini",
            structural_only=structural_only,
        )
    if seen != {"ini", "model"}:
        fail("Both ini and model blocks are required")
    program.dosing = list(program.states)
    return program


def pumas(text, *, structural_only=False, **_):
    program = Program()
    match = re.match(r"(?:(?:" + NAME + r")\s*=\s*)?@model\s+begin\b", text)
    if not match or not text.rstrip().endswith("end"):
        fail("Pumas import requires one @model begin ... end block")
    body = text[match.end() :].strip()[:-3].strip()
    sections, current = {}, None
    builtins = {
        "Central1": ["Central' = -CL/Vc*Central"],
        "Central1Periph1": [
            "Central' = -(CL+Q)/Vc*Central+Q/Vp*Peripheral",
            "Peripheral' = Q/Vc*Central-Q/Vp*Peripheral",
        ],
        "Depots1Central1": ["Depot' = -Ka*Depot", "Central' = Ka*Depot-CL/Vc*Central"],
        "Depots1Central1Periph1": [
            "Depot' = -Ka*Depot",
            "Central' = Ka*Depot-(CL+Q)/Vc*Central+Q/Vp*Peripheral",
            "Peripheral' = Q/Vc*Central-Q/Vp*Peripheral",
        ],
    }
    for line in body.splitlines():
        line = line.strip()
        if not line:
            continue
        match = re.fullmatch(
            r"@(param|pre|init|dynamics|vars|derived|observed|random|covariates)\s+begin",
            line,
        )
        if match and current is None:
            current = match[1]
            if current in sections:
                fail("Repeated Pumas block: " + current)
            sections[current] = []
        elif line == "end" and current:
            current = None
        elif current:
            sections[current].append(line)
        elif line.startswith("@covariates "):
            if "covariates" in sections:
                fail("Repeated Pumas covariates declaration")
            sections["covariates"] = line[len("@covariates ") :].split()
        elif line.startswith("@dynamics ") and line[len("@dynamics ") :] in builtins:
            if "dynamics" in sections:
                fail("Repeated Pumas dynamics declaration")
            sections["dynamics"] = builtins[line[len("@dynamics ") :]]
        else:
            fail("Unsupported Pumas block/statement: " + line)
    if current:
        fail("Unclosed Pumas block")
    for section, statements in sections.items():
        for statement in statements:
            if section == "param":
                match = re.fullmatch(
                    r"(" + NAME + r")\s*(?:∈|in)\s*RealDomain\(([^()]*)\)", statement
                )
                if not match:
                    fail("Pumas parameter declarations must use scalar RealDomain")
                fields = {}
                for field in split_top(match[2].lstrip(";"), separators=","):
                    item = re.fullmatch(r"(init|lower|upper)\s*=\s*(.+)", field)
                    if not item or item[1] in fields:
                        fail("Unsupported RealDomain option")
                    fields[item[1]] = number(item[2])
                if match[1] in program.parameters:
                    fail("Duplicate Pumas parameter declaration")
                program.parameters[match[1]] = fields.get("init")
                if "lower" in fields or "upper" in fields:
                    program.bounds[match[1]] = (
                        fields.get("lower"),
                        fields.get("upper"),
                    )
                program.metadata.append(statement)
            elif section == "random" or "~" in statement:
                statistical(statement, program, structural_only)
            elif section == "covariates":
                if not re.fullmatch(NAME, statement):
                    fail("Pumas covariates must name one scalar per line")
                program.parameters[statement] = None
            elif section == "dynamics":
                match = re.fullmatch(r"(" + NAME + r")'\s*=\s*(.+)", statement)
                if not match:
                    fail(
                        "Pumas @dynamics requires explicit ODEs; built-in dynamics and reactions are outside this subset"
                    )
                program.assign(*match.groups(), "states")
            else:
                target = "initials" if section == "init" else "definitions"
                assignment(statement.replace("@.", ""), program, target)
                if section in ("derived", "observed"):
                    name = ASSIGNMENT.fullmatch(statement.replace("@.", ""))[1]
                    program.outputs[name] = name
    # Julia vector indices need explicit scalar realizations: parameters={"eta_1": 0}.
    for section in (
        program.states,
        program.initials,
        program.definitions,
        program.outputs,
    ):
        for name, value in section.items():
            section[name] = re.sub(r"(" + NAME + r")\[(\d+)\]", r"\1_\2", value)
    program.dosing = list(program.states)
    return program


def monolix(text, *, structural_only=False, **_):
    program, current, input_names = Program(), None, []
    for line in text.splitlines():
        if not line:
            continue
        if line == "[LONGITUDINAL]":
            if current is not None:
                fail("Only one [LONGITUDINAL] section is supported")
            current = "input"
        elif line.endswith(":"):
            section = line[:-1]
            if section not in ("PK", "EQUATION", "OUTPUT"):
                fail("Unsupported Mlxtran section: " + section)
            current = section
        elif current == "input":
            match = re.fullmatch(r"input\s*=\s*\{([^{}]*)\}", line)
            if not match or input_names:
                fail("Expected one Mlxtran input={...} declaration")
            input_names = split_top(match[1], separators=",")
            for name in input_names:
                if not re.fullmatch(NAME, name) or name in program.parameters:
                    fail("Invalid/duplicate Mlxtran input name")
                program.parameters[name] = None
        elif current == "PK":
            match = re.fullmatch(r"depot\(\s*target\s*=\s*(" + NAME + r")\s*\)", line)
            if not match:
                fail(
                    "Direct Mlxtran import supports depot(target=state) only; absorption/lag macros require converter lowering"
                )
            program.dosing.append(match[1])
        elif current == "EQUATION":
            if line in ("odeType=stiff", "odeType = stiff"):
                program.metadata.append(line)
                continue
            match = ASSIGNMENT.fullmatch(line)
            if not match:
                fail("Unsupported Mlxtran equation: " + line)
            name, value = match.groups()
            if name == "t_0":
                if number(value) != 0:
                    fail("Nonzero Mlxtran t_0 requires explicit initial-time lowering")
            elif name.startswith("ddt_"):
                program.assign(name[4:], value, "states")
            elif name.endswith("_0"):
                program.assign(name[:-2], value, "initials")
            else:
                program.assign(name, value)
        elif current == "OUTPUT":
            match = re.fullmatch(r"output\s*=\s*(?:\{([^{}]*)\}|(" + NAME + r"))", line)
            if not match or program.outputs:
                fail("Expected one deterministic Mlxtran output declaration")
            names = split_top(match[1] or match[2], separators=",")
            program.outputs = {name: name for name in names}
        else:
            fail("Unsupported Mlxtran content: " + line)
    if not program.outputs:
        fail("Mlxtran OUTPUT section is required")
    return program


def phoenix(text, *, structural_only=False, **_):
    program = Program()
    match = re.match(r"(" + NAME + r")\s*\(\s*\)\s*", text)
    if not match:
        fail("Phoenix PML requires name(){...}")
    for statement in split_top(block(text[match.end() :]), separators="\n;"):
        call = re.fullmatch(
            r"(deriv|dosepoint|stparm|fixef|ranef|error|observe|covariate)\((.*)\)",
            statement,
            re.S,
        )
        if not call:
            assignment(statement, program)
            continue
        name, payload = call.groups()
        if name == "deriv":
            assignment(payload, program, "states")
        elif name == "dosepoint":
            if not re.fullmatch(NAME, payload.strip()):
                fail("PML dosepoint options require explicit lowering")
            program.dosing.append(payload.strip())
        elif name == "stparm":
            for part in split_top(payload, separators=",\n"):
                assignment(part, program)
        elif name == "fixef":
            for part in split_top(payload, separators=",\n"):
                match = re.fullmatch(r"(" + NAME + r")\s*=\s*c\(([^()]*)\)", part)
                if not match:
                    fail("Unsupported PML fixed-effect declaration")
                values = match[2].split(",")
                if len(values) != 3:
                    fail("PML fixef requires c(lower,initial,upper)")
                if match[1] in program.parameters:
                    fail("Repeated PML fixed-effect declaration")
                program.parameters[match[1]] = number(values[1])
                if values[0].strip() or values[2].strip():
                    program.bounds[match[1]] = (
                        number(values[0]) if values[0].strip() else None,
                        number(values[2]) if values[2].strip() else None,
                    )
                program.metadata.append(part)
        elif name == "covariate":
            if not re.fullmatch(NAME, payload.strip()):
                fail("Only scalar PML covariates are supported")
            program.parameters[payload.strip()] = None
        else:
            statistical(statement, program, structural_only)
    return program


def winnonlin(text, **_):
    program, current, counts = Program(), None, {}
    sections = {"COMMANDS", "TEMPORARY", "STARTING", "DIFFERENTIAL", "FUNCTION"}
    aliases = {}
    for line in text.upper().splitlines():
        if not line:
            continue
        heading = re.fullmatch(
            r"(COMMANDS|TEMPORARY|STARTING|DIFFERENTIAL|FUNCTION)(?:\s+(\d+))?", line
        )
        if heading:
            if current:
                fail("Nested WinNonlin blocks")
            current = heading[1]
            if current == "FUNCTION" and heading[2] not in (None, "1"):
                fail("Only one WinNonlin FUNCTION block is supported")
        elif line == "END":
            if current is None:
                fail("Unexpected WinNonlin END")
            current = None
        elif current == "COMMANDS":
            match = re.fullmatch(
                r"(NPARAMETERS|NCONSTANTS|NDERIVATIVES|NFUNCTIONS)\s*(?:=)?\s*(\d+)",
                line,
            )
            values = re.fullmatch(r"(INITIAL|CONSTANTS)\s*(?:=)?\s*(.+)", line)
            if match:
                if match[1] in counts:
                    fail("Duplicate WinNonlin dimension declaration")
                counts[match[1]] = int(match[2])
            elif values:
                kind = "P" if values[1] == "INITIAL" else "CON"
                parsed = [number(v) for v in re.split(r"[\s,]+", values[2]) if v]
                expected = counts.get("NPARAMETERS" if kind == "P" else "NCONSTANTS")
                if expected is None or len(parsed) != expected:
                    fail("WinNonlin value count differs from its dimension declaration")
                for i, value in enumerate(parsed, 1):
                    key = kind + "_" + str(i)
                    if key in program.parameters:
                        fail("Repeated WinNonlin value declaration")
                    aliases[kind, i] = key
                    program.parameters[key] = value
            else:
                fail("Unsupported WinNonlin command: " + line)
        elif current in sections:
            n = counts.get("NDERIVATIVES", 0)
            if n > 2000:
                fail("WinNonlin state count exceeds budget", code="resource_limit")
            aliases.update({("Z", i): "Z_" + str(i) for i in range(1, n + 1)})
            if current in ("STARTING", "DIFFERENTIAL"):
                kind = "Z" if current == "STARTING" else "DZ"
                match = re.fullmatch(kind + r"\((\d+)\)\s*=\s*(.+)", line)
                if not match or not 1 <= int(match[1]) <= n:
                    fail("WinNonlin equation must target a declared state")
                program.assign(
                    "Z_" + match[1],
                    _indexed(match[2], aliases, set()),
                    "initials" if current == "STARTING" else "states",
                )
            else:
                assignment(_indexed(line, aliases, set()), program)
        else:
            fail("Unsupported WinNonlin command/block: " + line)
    if current or counts.get("NFUNCTIONS", 1) != 1:
        fail("WinNonlin requires closed blocks and one FUNCTION")
    if set(program.states) != {
        "Z_" + str(i) for i in range(1, counts.get("NDERIVATIVES", 0) + 1)
    }:
        fail("Every WinNonlin derivative requires an equation")
    if "F" not in program.definitions:
        fail("WinNonlin FUNCTION must define F")
    program.outputs = {"F": "F"}
    return program


READERS = {
    "nonmem": nonmem,
    "nlmixr2": nlmixr2,
    "rxode2": rxode2,
    "pumas": pumas,
    "monolix": monolix,
    "phoenix": phoenix,
    "winnonlin": winnonlin,
}
