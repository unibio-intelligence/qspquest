"""Independent analytic PK and cross-format structural/event preservation checks."""

from copy import deepcopy
import math

import pytest
import qspquest as pf
from qspquest.adapters import (
    AdapterError,
    SOFTWARE,
    export_model,
    import_model,
    simulation_dataset,
    export_dataset,
    import_rxode2_snapshot,
)


SOURCES = {
    "nonmem": """$PROBLEM Synthetic oral model
$SUBROUTINES ADVAN13 TOL=9
$MODEL COMP=(DEPOT,DEFDOS) COMP=(CENTRAL,DEFOBS)
$THETA 1.1 FIX 0.3 FIX 3 FIX
$PK
KA=THETA(1)
CL=THETA(2)
V=THETA(3)
$DES
DADT(1)=-KA*A(1)
DADT(2)=KA*A(1)-CL/V*A(2)
$ERROR
Y=A(2)/V
""",
    "nlmixr2": """oral <- function() {
ini({
ka <- 1.1
cl <- 0.3
v <- 3
})
model({
d/dt(depot) = -ka*depot
d/dt(central) = ka*depot-cl/v*central
cp = central/v
})
}""",
    "rxode2": """rxode2({
d/dt(depot) = -ka*depot;
d/dt(central) = ka*depot-cl/v*central;
cp = central/v;
})""",
    "pumas": """@model begin
@param begin
ka ∈ RealDomain(init=1.1)
cl ∈ RealDomain(init=0.3)
v ∈ RealDomain(init=3)
end
@init begin
Depot = 0
Central = 0
end
@dynamics begin
Depot' = -ka*Depot
Central' = ka*Depot-cl/v*Central
end
@observed begin
cp = @. Central/v
end
end""",
    "monolix": """[LONGITUDINAL]
input = {ka, cl, v}
PK:
depot(target=depot)
EQUATION:
t_0 = 0
depot_0 = 0
central_0 = 0
ddt_depot = -ka*depot
ddt_central = ka*depot-cl/v*central
cp = central/v
OUTPUT:
output = {cp}
""",
    "phoenix": """oral(){
fixef(ka=c(0,1.1,), cl=c(0,0.3,), v=c(0,3,))
dosepoint(depot)
deriv(depot=-ka*depot)
deriv(central=ka*depot-cl/v*central)
cp=central/v
}""",
    "winnonlin": """COMMANDS
NPARAMETERS 3
INITIAL 1.1,0.3,3
NDERIVATIVES 2
NFUNCTIONS 1
END
STARTING
Z(1)=0
Z(2)=0
END
DIFFERENTIAL
DZ(1)=-P(1)*Z(1)
DZ(2)=P(1)*Z(1)-P(2)/P(3)*Z(2)
END
FUNCTION 1
F=Z(2)/P(3)
END""",
}


def imported(software):
    options = {}
    if software in ("rxode2", "monolix"):
        options["parameters"] = {"ka": 1.1, "cl": 0.3, "v": 3}
    if software in ("rxode2", "nlmixr2", "phoenix"):
        options["outputs"] = {"cp": "cp"}
    if software == "winnonlin":
        options["dosing_targets"] = ["Z_1"]
    return import_model(
        SOURCES[software], software, model_id="synthetic_oral", time_unit="h", **options
    )


def run(model, administrations=None, observations=None):
    target = next(s["name"] for s in model.model["states"] if s.get("dosing"))
    req = pf.run_request(
        end=8,
        times=[0, 0.25, 1, 2, 4, 8],
        time_unit="h",
        administrations=administrations or [pf.bolus(target, 6, "1")],
        rtol=1e-10,
        atol=1e-12,
    )
    if observations:
        req["regimen"]["observations"] = observations
        req["regimen"]["samples"] = []
    return model.compile().run(req)


def oral(time, amount=6):
    ka, k = 1.1, 0.1
    return amount / 3 * ka / (ka - k) * (math.exp(-k * time) - math.exp(-ka * time))


@pytest.mark.parametrize("software", SOFTWARE)
def test_each_import_matches_independent_oral_solution(software):
    model = imported(software)
    result = run(model)
    assert result["outputs"][0]["values"] == pytest.approx(
        [oral(t) for t in result["times"]], abs=2e-8
    )
    assert len(model.source_sha256) == 64
    assert model.model["time_unit"] == "h"
    assert model.warnings and model.compartment_map[1]


@pytest.mark.parametrize("software", SOFTWARE)
def test_export_reimport_preserves_trajectory_and_dose_targets(software):
    model = imported("nlmixr2")
    exported = export_model(model, software)
    restored = exported.import_back()
    assert run(restored)["outputs"][0]["values"] == pytest.approx(
        run(model)["outputs"][0]["values"], abs=2e-8
    )
    assert len([s for s in restored.model["states"] if s.get("dosing")]) == 2


@pytest.mark.parametrize("software", SOFTWARE)
def test_subject_rows_repeat_infusions_and_export_preserve_semantics(software):
    model = imported(software)
    canonical = [
        {
            "ID": "B",
            "TIME": 0,
            "EVID": 1,
            "CMT": 1,
            "AMT": 6,
            "RATE": 3,
            "ADDL": 1,
            "II": 4,
        },
        {"ID": "A", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 3},
        {"ID": "B", "TIME": 0, "EVID": 0, "DV": 0},
        {"ID": "B", "TIME": 2, "EVID": 0, "DV": 0.1},
        {"ID": "A", "TIME": 2, "EVID": 0, "DV": 0.2},
        {"ID": "B", "TIME": 2, "EVID": 0, "DV": 0.1},
        {"ID": "B", "TIME": 8, "EVID": 0, "DV": 0.1},
    ]
    columns = {
        n: "column_" + n.lower() for n in {key for row in canonical for key in row}
    }
    source = [{columns[n]: v for n, v in row.items()} for row in canonical]
    before = deepcopy(source)
    dataset = simulation_dataset(
        source, model, software=software, columns=columns, end=8, rtol=1e-10, atol=1e-12
    )
    results = dataset.run()["results"]
    assert source == before
    assert list(results) == ["B", "A"]
    assert len(dataset.requests["B"]["regimen"]["administrations"]) == 2
    assert export_dataset(dataset, software, columns=columns) == source
    assert results["A"]["outputs"][0]["values"][0] == pytest.approx(
        oral(2, 3), abs=2e-8
    )

    # Exact integral of the oral impulse response under two finite depot infusions.
    def integrated(time):
        k, ka = 0.1, 1.1
        return (
            ka
            / (3 * (ka - k))
            * ((1 - math.exp(-k * time)) / k - (1 - math.exp(-ka * time)) / ka)
        )

    expected = [
        sum(
            3 * (integrated(t - start) - integrated(max(0, t - start - 2)))
            for start in (0, 4)
            if t >= start
        )
        for t in results["B"]["times"]
    ]
    assert results["B"]["outputs"][0]["values"] == pytest.approx(expected, abs=5e-8)
    assert [r["source_row"] for r in dataset.rows] == list(range(1, 8))


def test_time_coordinate_nonzero_initials_and_pre_post_dose():
    source = "{\nx(0)=2;\nd/dt(x)=-k*x;\ny=x + t;\n}"
    for unit in ("s", "min", "h", "d"):
        model = import_model(
            source, "rxode2", time_unit=unit, parameters={"k": 0.2}, outputs={"y": "y"}
        )
        target = model.compartment_map[1]
        req = pf.run_request(
            end=2,
            times=[1],
            time_unit=unit,
            administrations=[pf.bolus(target, 3, "1", at=1, time_unit=unit)],
            rtol=1e-10,
            atol=1e-12,
        )
        req["regimen"]["samples"] = []
        req["regimen"]["observations"] = [
            {"time": pf.quantity(1, unit), "side": side} for side in ("pre", "post")
        ]
        result = model.compile().run(req)
        assert result["outputs"][0]["values"] == pytest.approx(
            [2 * math.exp(-0.2) + 1, 2 * math.exp(-0.2) + 4], abs=2e-8
        )


@pytest.mark.parametrize(
    "software,source",
    [
        ("nonmem", SOURCES["nonmem"].replace("Y=A(2)/V", "Y=A(2)/V+EPS(1)")),
        ("nonmem", SOURCES["nonmem"].replace("ADVAN13", "ADVAN2")),
        ("nonmem", SOURCES["nonmem"] + "$PRIOR NWPRI\n"),
        ("nlmixr2", SOURCES["nlmixr2"].replace("cp = central/v", "cp = linCmt()")),
        ("rxode2", "{\nd/dt(x) = delay(x,1);\n}"),
        ("rxode2", "{\nd/dt(x) = -x;\nx=4;\n}"),
        ("rxode2", "{\nd/dt(x) = -x;\ny=1;\ny=2;\n}"),
        (
            "pumas",
            SOURCES["pumas"].replace(
                "Depot' = -ka*Depot", "if t > 1\nDepot' = -ka*Depot\nend"
            ),
        ),
        (
            "monolix",
            SOURCES["monolix"].replace(
                "depot(target=depot)", "depot(target=depot, ka)"
            ),
        ),
        ("monolix", SOURCES["monolix"].replace("t_0 = 0", "t_0 = 2")),
        (
            "phoenix",
            SOURCES["phoenix"].replace("dosepoint(depot)", "dosepoint(depot, tlag=2)"),
        ),
        ("winnonlin", SOURCES["winnonlin"] + "\nTRANSFORM\nY=log(Y)\nEND"),
    ],
)
def test_unsupported_constructs_return_no_partial_model(software, source):
    parameters = {"ka": 1.1, "cl": 0.3, "v": 3} if software == "monolix" else None
    with pytest.raises((AdapterError, pf.QSPQuestError)):
        import_model(source, software, time_unit="h", parameters=parameters)


@pytest.mark.parametrize("bad", [float("nan"), float("inf"), True, "not-a-number"])
def test_bad_bindings_rejected(bad):
    with pytest.raises(AdapterError):
        import_model("{d/dt(x)=-k*x;}", "rxode2", time_unit="h", parameters={"k": bad})


def test_nonmem_f_requires_explicit_observation_compartment():
    source = SOURCES["nonmem"].replace("Y=A(2)/V", "S2=V\nY=F")
    model = import_model(source, "nonmem", time_unit="h")
    assert run(model)["outputs"][0]["values"] == pytest.approx(
        run(imported("nonmem"))["outputs"][0]["values"], abs=2e-8
    )
    with pytest.raises(AdapterError, match="DEFOBS"):
        import_model(source.replace(",DEFOBS", ""), "nonmem", time_unit="h")


def test_explicit_structural_only_preserves_statistical_declarations():
    source = (
        SOURCES["nlmixr2"]
        .replace("v <- 3", "v <- 3\neta.cl ~ 0.04")
        .replace("cl/v*central", "cl*exp(eta.cl)/v*central")
        .replace("cp = central/v", "cp = central/v\ncp ~ add(0.1)")
    )
    with pytest.raises(AdapterError):
        import_model(source, "nlmixr2", time_unit="h")
    with pytest.raises(AdapterError):
        import_model(source, "nlmixr2", time_unit="h", structural_only=True)
    model = import_model(
        source,
        "nlmixr2",
        time_unit="h",
        structural_only=True,
        parameters={"eta.cl": 0},
        outputs={"cp": "cp"},
    )
    assert model.source_metadata == ("eta.cl ~ 0.04", "cp ~ add(0.1)")
    assert run(model)["outputs"][0]["values"] == pytest.approx(
        run(imported("nlmixr2"))["outputs"][0]["values"], abs=2e-8
    )


@pytest.mark.parametrize(
    "changes",
    [{"SS": 1}, {"RATE": -1}, {"ADDL": 1.5}, {"EVID": 4}, {"CMT": 0}, {"CENS": 1}],
)
def test_invalid_table_semantics_rejected(changes):
    row = {"ID": "A", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6, **changes}
    with pytest.raises(AdapterError):
        simulation_dataset(
            [row, {"ID": "A", "TIME": 2, "EVID": 0}], imported("nlmixr2"), end=8
        )


def test_injection_and_limits_do_not_execute(tmp_path):
    target = tmp_path / "must-not-exist"
    source = "{d/dt(x)=__import__('pathlib').Path('" + str(target) + "').touch();}"
    with pytest.raises(AdapterError):
        import_model(source, "rxode2", time_unit="h")
    assert not target.exists()
    with pytest.raises(AdapterError, match="budget"):
        import_model("x" * 1000001, "rxode2", time_unit="h")


def test_optional_converter_snapshot_uses_same_native_importer():
    snapshot = {
        "schema": "qspquest.rxode2-ui/v1",
        "model_source": "{d/dt(x)=-k*x;y=x;}",
        "parameters": {"k": 0.2},
        "states": ["x"],
        "source_function": "synthetic fixture",
        "versions": {"rxode2": "synthetic-test"},
    }
    model = import_rxode2_snapshot(snapshot, time_unit="h", outputs={"y": "y"})
    expected = [6 * math.exp(-0.2 * t) for t in run(model)["times"]]
    assert run(model)["outputs"][0]["values"] == pytest.approx(expected, abs=2e-8)
    assert model.source_metadata[-1]["versions"] == snapshot["versions"]
    snapshot["states"] = ["wrong"]
    with pytest.raises(AdapterError, match="state order"):
        import_rxode2_snapshot(snapshot, time_unit="h")


def test_pumas_explicit_individual_effects_and_covariates():
    source = """@model begin
@param begin
tvcl ∈ RealDomain(init=0.3)
v ∈ RealDomain(init=3)
end
@random begin
eta ~ MvNormal(omega)
end
@covariates begin
weight
end
@pre begin
cl = tvcl*(weight/70)^0.75*exp(eta[1])
end
@dynamics begin
Central' = -cl/v*Central
end
@observed begin
cp = @. Central/v
end
end"""
    model = import_model(
        source,
        "pumas",
        time_unit="h",
        structural_only=True,
        parameters={"eta_1": 0.2, "weight": 80},
    )
    result = run(model)
    k = 0.1 * (80 / 70) ** 0.75 * math.exp(0.2)
    assert result["outputs"][0]["values"] == pytest.approx(
        [2 * math.exp(-k * t) for t in result["times"]], abs=2e-8
    )


def test_nonmem_explicit_effect_scaling_and_source_compartment_order():
    source = (
        SOURCES["nonmem"]
        .replace("CL=THETA(2)", "CL=THETA(2)*EXP(ETA(1))\nS2=V")
        .replace("Y=A(2)/V", "Y=F")
    )
    model = import_model(source, "nonmem", time_unit="h", parameters={"ETA_1": 0})
    assert run(model)["outputs"][0]["values"] == pytest.approx(
        [oral(t) for t in run(model)["times"]], abs=2e-8
    )


def test_nonzero_initial_export_is_preserved_or_rejected_explicitly():
    model = import_model(
        "{x(0)=2;d/dt(x)=-k*x;y=x;}",
        "rxode2",
        time_unit="h",
        parameters={"k": 0.2},
        outputs={"y": "y"},
    )
    for software in ("nlmixr2", "rxode2", "pumas", "monolix", "winnonlin"):
        restored = export_model(model, software).import_back()
        assert run(restored)["outputs"][0]["values"] == pytest.approx(
            run(model)["outputs"][0]["values"], abs=2e-8
        )
    for software in ("nonmem", "phoenix"):
        with pytest.raises(AdapterError, match="initial"):
            export_model(model, software)


def test_functions_and_scientific_notation_do_not_become_indexed_parameters():
    model = import_model(
        SOURCES["nonmem"].replace("CL=THETA(2)", "CL=THETA(2)*EXP(0)+1D-3*0"),
        "nonmem",
        time_unit="h",
    )
    assert run(model)["outputs"][0]["values"] == pytest.approx(
        [oral(t) for t in run(model)["times"]], abs=2e-8
    )


def test_source_parameter_bounds_are_enforced():
    source = SOURCES["pumas"].replace("init=0.3", "lower=0.1, init=0.3, upper=1")
    with pytest.raises(AdapterError, match="bounds"):
        import_model(source, "pumas", time_unit="h", parameters={"cl": 0.01})
    model = import_model(source, "pumas", time_unit="h")
    cl = model.symbol_map["cl"]
    parameter = next(p for p in model.model["parameters"] if p["name"] == cl)
    assert [q["value"] for q in parameter["bounds"]] == [0.1, 1]
    for software in ("pumas", "nonmem", "phoenix"):
        restored = export_model(model, software).import_back()
        bounds = [p["bounds"] for p in restored.model["parameters"] if p.get("bounds")]
        assert [{"value": 0.1, "unit": "1"}, {"value": 1, "unit": "1"}] in bounds
    with pytest.raises(AdapterError, match="bounds"):
        export_model(model, "nlmixr2")


def test_dose_side_in_data_adapter_is_independent_of_row_order():
    model = import_model(
        "{d/dt(x)=-k*x;y=x;}",
        "rxode2",
        time_unit="h",
        parameters={"k": 0.2},
        outputs={"y": "y"},
    )
    rows = [
        {"ID": "A", "TIME": 0, "EVID": 0},
        {"ID": "A", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6},
    ]
    before = simulation_dataset(rows, model, end=1, observation_side="pre").run()
    after = simulation_dataset(rows, model, end=1, observation_side="post").run()
    assert before["results"]["A"]["outputs"][0]["values"] == [0]
    assert after["results"]["A"]["outputs"][0]["values"] == [6]


@pytest.mark.parametrize("software", SOFTWARE)
def test_nonlinear_export_matches_independent_implicit_solution(software):
    model = import_model(
        "{d/dt(central)=-vmax*central/(km+central);cp=central;}",
        "rxode2",
        time_unit="h",
        parameters={"vmax": 0.2, "km": 1},
        outputs={"cp": "cp"},
    )
    restored = export_model(model, software).import_back()
    result = run(restored)
    for time, amount in zip(result["times"], result["outputs"][0]["values"]):
        # Integrated Michaelis-Menten equation, independent of the ODE solver.
        recovered_time = (6 - amount) / 0.2 + 1 / 0.2 * math.log(6 / amount)
        assert abs(recovered_time - time) < 2e-7


@pytest.mark.parametrize(
    "dynamics",
    ["Central1", "Central1Periph1", "Depots1Central1", "Depots1Central1Periph1"],
)
def test_pumas_linear_dynamics_against_independent_matrix_exponential(dynamics):
    import numpy as np
    from scipy.linalg import expm

    source = """@model begin
@param begin
CL ∈ RealDomain(init=0.3)
Vc ∈ RealDomain(init=3)
Ka ∈ RealDomain(init=1.1)
Q ∈ RealDomain(init=0.2)
Vp ∈ RealDomain(init=4)
end
@covariates weight
@dynamics DYNAMICS
@observed begin
cp = @. Central/Vc
end
end""".replace(
        "DYNAMICS", dynamics
    )
    model = import_model(source, "pumas", time_unit="h", parameters={"weight": 70})
    has_depot, has_peripheral = dynamics.startswith("Depots"), "Periph" in dynamics
    n = 1 + int(has_depot) + int(has_peripheral)
    central = int(has_depot)
    matrix = np.zeros((n, n))
    matrix[central, central] = -0.1
    if has_depot:
        matrix[0, 0], matrix[central, 0] = -1.1, 1.1
    if has_peripheral:
        matrix[central, central] -= 0.2 / 3
        matrix[central, -1], matrix[-1, central], matrix[-1, -1] = (
            0.2 / 4,
            0.2 / 3,
            -0.2 / 4,
        )
    initial = np.zeros(n)
    initial[0] = 6
    result = run(model)
    expected = [(expm(matrix * t) @ initial)[central] / 3 for t in result["times"]]
    assert result["outputs"][0]["values"] == pytest.approx(expected, abs=2e-8)


if __name__ == "__main__":
    import sys
    from pathlib import Path

    destination = Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=True)
    for software in ("rxode2", "nlmixr2"):
        exported = export_model(imported("nlmixr2"), software)
        (destination / (software + "-export.txt")).write_text(exported.source)
