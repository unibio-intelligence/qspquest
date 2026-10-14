"""Explicit dose/observation tables, shared across external software adapters."""

from copy import deepcopy
from dataclasses import dataclass
import math

from ._core import AdaptedModel, SOFTWARE, fail, number


COLUMNS = ("ID", "TIME", "EVID", "DV", "MDV", "AMT", "CMT", "RATE", "ADDL", "II", "SS")


@dataclass
class SimulationDataset:
    imported_model: AdaptedModel
    requests: dict
    rows: tuple
    source_software: str

    def run(self):
        """Compile once; return each subject's full native result and source mapping."""
        compiled = self.imported_model.compile()
        return {
            "results": {
                name: compiled.run(request) for name, request in self.requests.items()
            },
            "mapping": deepcopy(self.rows),
        }


def simulation_dataset(
    data,
    imported,
    *,
    software=None,
    columns=None,
    end,
    compartment_map=None,
    observation_side="post",
    rtol=1e-8,
    atol=1e-10,
):
    """Convert explicit NONMEM-style EVID 0/1 rows into subject run requests.

    Pumas uses lowercase default headers. Phoenix/WinNonlin/Monolix require an
    explicit role-to-column map, including EVID; no missing-dose inference.
    Dose amounts are numeric coordinates (unit 1), not inferred physical units.
    """
    if not isinstance(imported, AdaptedModel):
        fail("Expected an AdaptedModel", code="invalid_input")
    software = software or imported.software
    if software not in SOFTWARE or observation_side not in ("pre", "post"):
        fail("Unknown software or observation side", code="invalid_input")
    if columns is None:
        if software in ("monolix", "phoenix", "winnonlin"):
            fail(
                "This software requires an explicit role-to-column mapping",
                code="invalid_input",
            )
        columns = {n: n.lower() if software == "pumas" else n for n in COLUMNS}
    if (
        set(columns) - set(COLUMNS)
        or not {"ID", "TIME", "EVID"} <= set(columns)
        or len(set(columns.values())) != len(columns.values())
    ):
        fail("Invalid or ambiguous role-to-column mapping", code="invalid_input")
    if hasattr(data, "to_dict"):
        if len(data.columns) != len(set(data.columns)):
            fail("Duplicate table headers", code="invalid_input")
        data = data.to_dict(orient="records")
    rows = list(data)
    if not 1 <= len(rows) <= 50000:
        fail("Event table must contain 1 to 50000 rows", code="resource_limit")
    horizon, rtol, atol = number(end, "end"), number(rtol, "rtol"), number(atol, "atol")
    if horizon < 0 or rtol <= 0 or atol <= 0:
        fail("Invalid horizon/tolerances", code="invalid_input")
    valid_targets = {s["name"] for s in imported.model["states"] if s.get("dosing")}
    targets = (
        dict(compartment_map)
        if compartment_map is not None
        else {i: n for i, n in imported.compartment_map.items() if n in valid_targets}
    )
    if set(targets.values()) - valid_targets:
        fail("Compartment map includes a state without dosing support")
    groups, provenance, unit = {}, [], imported.model["time_unit"]
    total_doses = 0
    for source_row, row in enumerate(rows, 1):
        if not isinstance(row, dict) or set(row) - set(columns.values()):
            fail("Row has unsupported or unmapped columns: " + str(source_row))
        canonical = {
            role: row[column] for role, column in columns.items() if column in row
        }
        if not {"ID", "TIME", "EVID"} <= set(canonical):
            fail("Missing ID/TIME/EVID in row " + str(source_row), code="invalid_input")
        identity = canonical["ID"]
        if (
            isinstance(identity, bool)
            or identity is None
            or isinstance(identity, (dict, list))
            or isinstance(identity, float)
            and not math.isfinite(identity)
            or not str(identity)
        ):
            fail("Invalid subject identifier", code="invalid_input")
        identity = str(identity)
        time, event = number(canonical["TIME"]), number(canonical["EVID"])
        if not 0 <= time <= horizon or event not in (0, 1):
            fail("Time is outside horizon or EVID is outside 0/1")
        mdv = number(canonical.get("MDV", 0))
        if mdv not in (0, 1) or number(canonical.get("SS", 0)) != 0:
            fail("MDV must be 0/1 and steady-state rows are unsupported")
        group = groups.setdefault(identity, {"doses": [], "observations": []})
        mapped = {
            "source_row": source_row,
            "subject_id": identity,
            "record": deepcopy(canonical),
            "result_row": None,
            "side": None,
        }
        if event == 1:
            cmt = number(canonical.get("CMT", 0))
            if cmt != int(cmt) or int(cmt) not in targets:
                fail("Dose CMT lacks an explicit dosing target")
            amount, rate = number(canonical.get("AMT", 0)), number(
                canonical.get("RATE", 0)
            )
            additional, interval = number(canonical.get("ADDL", 0)), number(
                canonical.get("II", 0)
            )
            if (
                amount <= 0
                or rate < 0
                or additional != int(additional)
                or not 0 <= additional <= 10000
            ):
                fail(
                    "Invalid amount, rate or repeat count; modeled RATE codes are unsupported"
                )
            if (additional and interval <= 0) or (not additional and interval != 0):
                fail("II must be positive only with ADDL")
            delivery = (
                {"kind": "bolus"}
                if rate == 0
                else {
                    "kind": "infusion",
                    "span": {
                        "kind": "duration",
                        "duration": {"value": amount / rate, "unit": unit},
                    },
                }
            )
            if (
                time + int(additional) * interval > horizon
                or rate
                and time + int(additional) * interval + amount / rate > horizon
            ):
                fail("Dose or infusion extends beyond the explicit horizon")
            total_doses += int(additional) + 1
            if total_doses > 50000:
                fail("Expanded dose count exceeds 50000", code="resource_limit")
            for i in range(int(additional) + 1):
                group["doses"].append(
                    {
                        "target": targets[int(cmt)],
                        "time": {"value": time + i * interval, "unit": unit},
                        "amount": {"value": amount, "unit": "1"},
                        "delivery": deepcopy(delivery),
                        "bioavailability": 1.0,
                    }
                )
        else:
            if any(
                number(canonical.get(n, 0)) != 0 for n in ("AMT", "RATE", "ADDL", "II")
            ):
                fail("Observation row contains dose fields")
            if not mdv:
                mapped["result_row"] = len(group["observations"])
                mapped["side"] = observation_side
                group["observations"].append(
                    {"time": {"value": time, "unit": unit}, "side": observation_side}
                )
        provenance.append(mapped)
    requests = {}
    for identity, group in groups.items():
        if not group["observations"]:
            fail("Every subject requires an observation")
        times = [o["time"]["value"] for o in group["observations"]]
        if times != sorted(times):
            fail("Observation times must be nondecreasing within each subject")
        requests[identity] = {
            "schema": "qspquest.run/v0.1",
            "solver": "diffsol_bdf",
            "budgets": {
                "solver_callbacks": 1000000,
                "events": 100000,
                "output_values": 1000000,
            },
            "regimen": {
                "end": {"value": horizon, "unit": unit},
                "samples": [],
                "administrations": group["doses"],
                "observations": group["observations"],
                "rtol": rtol,
                "atol": atol,
            },
        }
    return SimulationDataset(imported, requests, tuple(provenance), software)


def export_dataset(dataset, software, *, columns=None):
    """Return source-order records with explicit headers; no vendor project files."""
    if not isinstance(dataset, SimulationDataset) or software not in SOFTWARE:
        fail("Expected a SimulationDataset and software name", code="invalid_input")
    if columns is None:
        if software in ("monolix", "phoenix", "winnonlin"):
            fail("Explicit destination column mapping required", code="invalid_input")
        columns = {n: n.lower() if software == "pumas" else n for n in COLUMNS}
    used = {n for row in dataset.rows for n in row["record"]}
    if (
        set(columns) - set(COLUMNS)
        or used - set(columns)
        or len(set(columns.values())) != len(columns.values())
    ):
        fail("Destination mapping would lose or conflate source columns")
    return [
        {columns[n]: deepcopy(value) for n, value in row["record"].items()}
        for row in dataset.rows
    ]
