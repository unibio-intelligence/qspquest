"""Run a typed, synthetic one-compartment trajectory; optionally make a frame."""
import argparse
import math

from qspquest import AnalysisResult, CompiledModel, RunRequest, bolus, run_request

SOURCE = """model typed_one_compartment
time_unit = h
parameters
  cl = 0.3 [L/h] log
  v = 3 [L] log
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


def simulate() -> AnalysisResult:
    request: RunRequest = run_request(
        end=12.0,
        times=[0.0, 1.0, 2.0, 4.0, 8.0, 12.0],
        administrations=[bolus("central", 6.0, "mg")],
        solver="diffsol_bdf",
        rtol=1e-10,
        atol=1e-12,
    )
    result = CompiledModel(SOURCE).run(request)
    assert isinstance(result, dict)
    assert len(result.rows()) == 6
    for row in result.rows():
        assert row["output"] == "cp" and row["unit"] == "mg/L"
        assert row["side"] == "post" and row["time_unit"] == "h"
        expected = 2.0 * math.exp(-0.1 * row["time"])
        assert abs(row["value"] - expected) < 1e-8
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frame", action="store_true", help="requires qspquest[notebook]")
    args = parser.parse_args()
    result = simulate()
    print(result.summary())
    print(f"Verified {len(result.rows())} analytical concentration samples.")
    if args.frame:
        frame = result.to_frame()
        assert len(frame) == 6
        print(frame.to_string(index=False))


if __name__ == "__main__":
    main()
