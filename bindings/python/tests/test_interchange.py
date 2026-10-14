"""Independent scientific references and explicit interchange rejection cases."""

from copy import deepcopy
from hashlib import sha256
from io import BytesIO
import json, math, stat, zipfile
from pathlib import Path
import xml.etree.ElementTree as ET
import pytest
import libsbml
import qspquest as pf
from qspquest.interchange import (
    InterchangeError,
    export_sbml,
    import_sbml,
    validate_sbml,
    export_omex,
    import_omex,
    NS,
    SEDML_NS,
)

ROOT = Path(__file__).resolve().parents[3]
MODEL = json.loads((ROOT / "examples/synthetic/absorption.model.json").read_text())


def request():
    # Coincident infusion stop and bolus, pre/post observations, lag and F.
    result = pf.run_request(
        end=8,
        times=[0, 1, 2, 4, 8],
        administrations=[
            pf.infusion("central", 100, "mg", duration=2),
            pf.bolus("central", 50, "mg", at=1, lag=1, bioavailability=0.8),
        ],
        rtol=1e-10,
        atol=1e-12,
    )
    result["regimen"]["observations"] = [
        {"time": pf.quantity(t, "h"), "side": s}
        for t, s in [
            (0, "post"),
            (1, "post"),
            (2, "pre"),
            (2, "post"),
            (4, "post"),
            (8, "post"),
        ]
    ]
    return result


def test_sbml_and_omex_preserve_native_units_and_event_sides():
    model = import_sbml(export_sbml(MODEL))
    assert model.model == MODEL and not model.warnings
    source = export_omex(model, {"coincident": request()})
    archive = import_omex(source, expected_sha256=sha256(source).hexdigest())
    assert export_omex(archive.imported_model, archive.scenarios) == source
    answer = archive.run("coincident")
    k = (
        MODEL["parameters"][0]["default"]["value"]
        / MODEL["parameters"][1]["default"]["value"]
    )
    v = MODEL["parameters"][1]["default"]["value"]
    for t, side, c in zip(
        answer["times"], answer["sides"], answer["outputs"][0]["values"]
    ):
        expected = (
            50 / (v * k) * (1 - math.exp(-k * min(t, 2))) * math.exp(-k * max(0, t - 2))
        )
        if t > 2 or (t == 2 and side == "post"):
            expected += 40 / v * math.exp(-k * (t - 2))
        assert abs(c - expected) < 1e-7
    assert archive.run("coincident")["identity"] == answer["identity"]


def _plain_sbml(time_units="second"):
    doc = libsbml.SBMLDocument(3, 2)
    m = doc.createModel()
    m.setId("external_decay")
    m.setTimeUnits(time_units)
    c = m.createCompartment()
    c.setId("volume")
    c.setSize(1)
    c.setConstant(True)
    c.setSpatialDimensions(0)
    s = m.createSpecies()
    s.setId("x")
    s.setCompartment("volume")
    s.setInitialAmount(10)
    s.setHasOnlySubstanceUnits(True)
    s.setBoundaryCondition(False)
    s.setConstant(False)
    p = m.createParameter()
    p.setId("k")
    p.setValue(0.1)
    p.setConstant(True)
    p = m.createParameter()
    p.setId("twice")
    p.setConstant(False)
    a = m.createAssignmentRule()
    a.setVariable("twice")
    a.setMath(libsbml.parseL3Formula("2*k"))
    r = m.createRateRule()
    r.setVariable("x")
    r.setMath(libsbml.parseL3Formula("-twice*x"))
    return doc


def test_unannotated_assignment_rules_and_time_coordinates():
    doc = _plain_sbml("source_hour")
    m = doc.getModel()
    definition = m.createUnitDefinition()
    definition.setId("source_hour")
    u = definition.createUnit()
    u.setKind(libsbml.UNIT_KIND_SECOND)
    u.setExponent(1)
    u.setMultiplier(3600)
    u.setScale(0)
    imported = import_sbml(libsbml.writeSBMLToString(doc))
    assert imported.warnings and imported.model["time_unit"] == "h"
    answer = imported.compile().run(
        pf.run_request(end=8, times=[0, 1, 8], rtol=1e-10, atol=1e-12)
    )
    assert answer["outputs"][0]["unit"] == "1"
    assert (
        max(
            abs(c - 10 * math.exp(-0.2 * t))
            for c, t in zip(answer["outputs"][0]["values"], answer["times"])
        )
        < 1e-7
    )


def _eval(node, values):
    if node.isNumber():
        return node.getValue()
    if node.isName():
        return values[node.getName()]
    args = [_eval(node.getChild(i), values) for i in range(node.getNumChildren())]
    if node.getType() == libsbml.AST_TIMES:
        return math.prod(args)
    if node.getType() == libsbml.AST_PLUS:
        return sum(args)
    if node.getType() == libsbml.AST_MINUS:
        return -args[0] if len(args) == 1 else args[0] - args[1]
    if node.getType() == libsbml.AST_DIVIDE:
        return args[0] / args[1]
    raise AssertionError(node.getType())


def test_sbml_math_uses_native_unit_scaling_and_initial_conversion():
    model = deepcopy(MODEL)
    model["parameters"][0]["default"] = {"value": math.e, "unit": "L/h"}
    model["parameters"][1]["default"] = {"value": 31500.39230874794, "unit": "mL"}
    model["states"][1]["initial"]["quantity"] = {"value": 0.1, "unit": "g"}
    text = export_sbml(model)
    m = libsbml.readSBMLFromString(text).getModel()
    values = {
        p.getId(): p.getValue() for p in m.getListOfParameters() if p.getConstant()
    }
    values.update(depot=0, central=100)
    # Independent evaluation of exported MathML, not the embedded JSON path.
    cp = next(r for r in m.getListOfRules() if r.getVariable() == "cp")
    assert abs(_eval(cp.getMath(), values) - 100 / 31.50039230874794) < 1e-12
    assert m.getSpecies("central").getInitialAmount() == 100
    rate = next(r for r in m.getListOfRules() if r.getVariable() == "central")
    assert (
        abs(_eval(rate.getMath(), values) + 2.718281828459045 / 31.50039230874794 * 100)
        < 1e-10
    )


@pytest.mark.parametrize(
    "kind",
    [
        "reaction",
        "event",
        "algebraic",
        "initial",
        "function",
        "boundary",
        "undefined_time",
        "delay",
    ],
)
def test_unsupported_sbml_semantics_are_rejected(kind):
    doc = _plain_sbml()
    m = doc.getModel()
    if kind == "reaction":
        r = m.createReaction()
        r.setId("reaction")
        r.setReversible(False)
        reactant = r.createReactant()
        reactant.setSpecies("x")
        reactant.setStoichiometry(1)
        reactant.setConstant(True)
        r.createKineticLaw().setMath(libsbml.parseL3Formula("k*x"))
        m.removeRule(1)
    elif kind == "event":
        e = m.createEvent()
        e.setUseValuesFromTriggerTime(True)
        t = e.createTrigger()
        t.setPersistent(True)
        t.setInitialValue(False)
        t.setMath(libsbml.parseL3Formula("time > 1"))
        a = e.createEventAssignment()
        a.setVariable("x")
        a.setMath(libsbml.parseL3Formula("2"))
    elif kind == "algebraic":
        m.createAlgebraicRule().setMath(libsbml.parseL3Formula("x-2"))
    elif kind == "initial":
        a = m.createInitialAssignment()
        a.setSymbol("k")
        a.setMath(libsbml.parseL3Formula(".1"))
    elif kind == "function":
        f = m.createFunctionDefinition()
        f.setId("f")
        f.setMath(libsbml.parseL3Formula("lambda(a,a*2)"))
    elif kind == "boundary":
        m.getSpecies("x").setBoundaryCondition(True)
    elif kind == "undefined_time":
        m.unsetTimeUnits()
    elif kind == "delay":
        m.getRule(1).setMath(libsbml.parseL3Formula("-k*delay(x,1)"))
    with pytest.raises(InterchangeError):
        import_sbml(libsbml.writeSBMLToString(doc))


def _rewrite(data, mutate, reindex=True):
    with zipfile.ZipFile(BytesIO(data)) as z:
        members = {n: z.read(n) for n in z.namelist()}
    mutate(members)
    if reindex:
        index = json.loads(members["index.json"])
        index["members"] = {
            n: sha256(b).hexdigest()
            for n, b in members.items()
            if n not in ("index.json", "manifest.xml")
        }
        members["index.json"] = json.dumps(index).encode()
    buffer = BytesIO()
    with zipfile.ZipFile(buffer, "w") as z:
        for n, b in members.items():
            z.writestr(n, b)
    return buffer.getvalue()


def test_tampered_math_annotation_and_trusted_digest():
    text = export_sbml(MODEL)
    tree = ET.fromstring(text)
    parameter = tree.find(".//{http://www.sbml.org/sbml/level3/version2/core}parameter")
    parameter.set("value", "100")
    with pytest.raises(InterchangeError, match="annotation"):
        import_sbml(ET.tostring(tree, encoding="unicode"))
    data = export_omex(MODEL, {"test": request()})
    with pytest.raises(InterchangeError, match="trusted digest"):
        import_omex(data, expected_sha256="0" * 64)

    def mutate_model(members):
        model = ET.fromstring(members["model.xml"])
        parameter = model.find(
            ".//{http://www.sbml.org/sbml/level3/version2/core}parameter"
        )
        parameter.set("value", str(float(parameter.get("value")) + 1))
        members["model.xml"] = ET.tostring(model)

    changed = _rewrite(data, mutate_model, False)
    with pytest.raises(InterchangeError, match="SHA-256"):
        import_omex(changed)


def test_rehashed_arbitrary_sedml_is_still_rejected():
    data = export_omex(MODEL, {"test": request()})

    def mutate(members):
        name = next(n for n in members if n.endswith(".sedml"))
        root = ET.fromstring(members[name])
        ET.SubElement(root, "{" + SEDML_NS + "}listOfTasks")
        members[name] = ET.tostring(root)

    with pytest.raises(InterchangeError, match="SED-ML"):
        import_omex(_rewrite(data, mutate))


@pytest.mark.parametrize(
    "name,mode",
    [
        ("../escape", stat.S_IFREG),
        ("/absolute", stat.S_IFREG),
        ("a\\b", stat.S_IFREG),
        ("symlink", stat.S_IFLNK),
    ],
)
def test_archive_never_extracts_unsafe_members(name, mode):
    buffer = BytesIO()
    with zipfile.ZipFile(buffer, "w") as z:
        info = zipfile.ZipInfo(name)
        info.filename = name
        info.external_attr = (mode | 0o644) << 16
        z.writestr(info, b"x")
    with pytest.raises(InterchangeError, match="Unsafe"):
        import_omex(buffer.getvalue())


def test_archive_checks_original_member_name_on_windows(monkeypatch):
    buffer = BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        info = zipfile.ZipInfo("a/b")
        info.filename = "a\\b"
        archive.writestr(info, b"x")
    # Reproduce ZipInfo's Windows normalization on every test platform.
    monkeypatch.setattr(zipfile.os, "sep", "\\")
    with pytest.raises(InterchangeError, match="Unsafe"):
        import_omex(buffer.getvalue())


def test_native_admission_rejects_bad_units_without_running():
    run = request()
    run["regimen"]["administrations"][0]["amount"]["unit"] = "L"
    with pytest.raises(InterchangeError, match="Native scenario admission"):
        export_omex(MODEL, {"bad": run})


def test_dtd_and_invalid_xml_rejected():
    with pytest.raises(InterchangeError, match="DTD"):
        import_sbml('<!DOCTYPE sbml [<!ENTITY x "x">]><sbml/>')
    with pytest.raises(InterchangeError):
        validate_sbml("<sbml>")


def test_native_archive_requires_current_schemas_without_mutation():
    model, run = deepcopy(MODEL), request()
    before = deepcopy((model, run))
    current_bytes = export_omex(model, {'dose': run})
    assert (model, run) == before
    archive = import_omex(current_bytes)
    assert archive.imported_model.model['schema'] == 'qspquest.model/v0.1'
    assert archive.scenarios['dose']['schema'] == 'qspquest.run/v0.1'
    for document in (model, run):
        tag = document['schema']
        for invalid in (tag.replace('qspquest.', 'other.'), tag.replace('v0.1', 'v99'), None, [tag], 1):
            document['schema'] = invalid
            rejected_input = deepcopy((model, run))
            with pytest.raises((InterchangeError, pf.QSPQuestError)):
                export_omex(model, {'dose': run})
            assert (model, run) == rejected_input
        del document['schema']
        with pytest.raises((InterchangeError, pf.QSPQuestError)):
            export_omex(model, {'dose': run})
        document['schema'] = tag
