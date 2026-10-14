"""Installation self-check runnable without compilers or optional analysis tools."""

import argparse
import importlib.metadata
import json
import math
import platform
import sys


def check_environment(*, analysis=False, interchange=False):
    """Check native loading, units/events and an independent analytic trajectory."""
    report = {
        "schema": "qspquest.doctor/v0.1",
        "python": sys.version.split()[0],
        "platform": platform.platform(),
        "machine": platform.machine(),
        "checks": [],
    }

    def check(name, fn):
        try:
            detail = fn()
            report["checks"].append(
                {"name": name, "status": "passed", "detail": detail}
            )
        except Exception as e:
            report["checks"].append(
                {"name": name, "status": "failed", "detail": str(e)}
            )

    def native():
        from . import __version__

        return {"qspquest": __version__}

    check("native_import", native)

    def simulation():
        from . import CompiledModel, bolus, run_request

        source = """model doctor_linear
time_unit = h
parameters
  cl = 1 [L/h] log
  v = 10 [L] log
states
  central = 0 [mg]
dynamics
  d/dt(central) = input_central - cl / v * central
outputs
  cp = central / v [mg/L]
dosing
  central [mg] = 1
end
"""
        result = CompiledModel(source).run(
            run_request(
                end=8,
                times=[0, 1, 4, 8],
                administrations=[bolus("central", 100, "mg")],
                solver="diffsol_bdf",
                rtol=1e-10,
                atol=1e-12,
            )
        )
        c = result["outputs"][0]
        errors = [
            abs(v - 10 * math.exp(-0.1 * t))
            for t, v in zip(result["times"], c["values"])
        ]
        if c["unit"] != "mg/L" or result["sides"] != ["post"] * 4 or max(errors) > 1e-7:
            raise RuntimeError(
                "native simulation differs from analytic bolus reference"
            )
        return {"max_absolute_error": max(errors), "identity": result["identity"]}

    check("analytic_simulation", simulation)
    if analysis:

        def analysis_tools():
            from ._analysis import dependencies

            np, _, special, _ = dependencies()
            if not np.isfinite(special.log_ndtr(-40)):
                raise RuntimeError("censored likelihood numerical dependency failed")
            return {m: importlib.metadata.version(m) for m in ["numpy", "scipy"]}

        check("analysis_dependencies", analysis_tools)
    if interchange:

        def interchange_tools():
            from .interchange import export_omex, import_omex, validate_sbml
            from . import run_request

            native = {
                "schema": "qspquest.model/v0.1",
                "model_id": "doctor_interchange",
                "description": "Independent numeric decay check",
                "time_unit": "s",
                "parameters": [],
                "states": [
                    {
                        "name": "x",
                        "unit": "1/s",
                        "initial": {
                            "kind": "quantity",
                            "quantity": {"value": 1, "unit": "1/s"},
                        },
                        "rhs": {
                            "kind": "binary",
                            "operator": "multiply",
                            "arguments": [
                                {
                                    "kind": "unary",
                                    "operator": "negative",
                                    "arguments": [{"kind": "symbol", "name": "x"}],
                                },
                                {"kind": "symbol", "name": "x"},
                            ],
                        },
                        "dosing": None,
                    }
                ],
                "outputs": [
                    {
                        "name": "value",
                        "unit": "1/s",
                        "expression": {"kind": "symbol", "name": "x"},
                    }
                ],
            }
            request = run_request(
                end=1, times=[0, 1], time_unit="s", rtol=1e-10, atol=1e-12
            )
            archive = import_omex(export_omex(native, {"decay": request}))
            values = archive.run("decay")["outputs"][0]["values"]
            if abs(values[-1] - 0.5) > 1e-7:
                raise RuntimeError(
                    "interchange differs from independent dx/dt=-x*x solution"
                )
            import libsbml

            return {
                "libsbml": libsbml.getLibSBMLDottedVersion(),
                "profile": archive.profile,
                "round_trip": "passed",
            }

        check("interchange_round_trip", interchange_tools)
    report["status"] = (
        "passed" if all(c["status"] == "passed" for c in report["checks"]) else "failed"
    )
    return report


def main(argv=None):
    """CLI entry point: qspquest-doctor [--analysis] [--interchange] [--json]."""
    parser = argparse.ArgumentParser(
        description="Check a local QSPQuest installation; no network or credentials are used."
    )
    parser.add_argument(
        "--analysis",
        action="store_true",
        help="also require the optional NumPy/SciPy analysis layer",
    )
    parser.add_argument(
        "--json", action="store_true", help="emit a machine-readable report"
    )
    parser.add_argument(
        "--interchange",
        action="store_true",
        help="also validate SBML/OMEX and an independent round-trip trajectory",
    )
    args = parser.parse_args(argv)
    report = check_environment(analysis=args.analysis, interchange=args.interchange)
    if args.json:
        print(json.dumps(report, indent=2, allow_nan=False))
    else:
        print(f"QSPQuest installation: {report['status']}")
        for c in report["checks"]:
            print(
                f"  {c['name']}: {c['status']}"
                + (f" — {c['detail']}" if c["status"] == "failed" else "")
            )
        print(
            "This checks installation and a small analytic case, not broad scientific qualification."
        )
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
