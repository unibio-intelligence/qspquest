"""Nonnegative invariants accept integration error within the absolute tolerance."""

import math
import unittest

import qspquest as qq

MODEL = """model depleted_depot
 time_unit = h
parameters
 cl = 2.4 [L/h] log estimated
 v = 60 [L] log estimated
 ka = 0.9 [1/h] log estimated
linear_pk
 amount_unit = mg
 clearance = cl
 compartment central volume v = 0 [mg]
 depot depot absorption ka = 0 [mg]
outputs
 cp = central / v [mg/L]
 depot_amount = depot [mg]
end
"""


def exact(t, cl=2.4, v=60.0, ka=0.9, dose=210.0):
    k = cl / v
    return dose * ka / (v * (ka - k)) * (math.exp(-k * t) - math.exp(-ka * t))


def request(solver, end=72, times=(1, 12, 24, 48, 72)):
    return qq.run_request(end=end, times=list(times), solver=solver,
                          administrations=[qq.bolus("depot", 300, "mg", bioavailability=0.7)])


class InvariantToleranceTest(unittest.TestCase):
    def test_bdf_completes_after_the_depot_is_depleted(self):
        model = qq.CompiledModel(MODEL)
        for end in (72, 240):
            times = (1, 12, 24, 48, end)
            bdf = model.run(request("diffsol_bdf", end, times))
            analytic = model.run(request("analytic_linear_pk", end, times))
            outputs = {c["name"]: c["values"] for c in bdf["outputs"]}
            reference = {c["name"]: c["values"] for c in analytic["outputs"]}
            for t, value, expected in zip(bdf["times"], outputs["cp"], reference["cp"]):
                self.assertLess(abs(value - expected), 1e-7)
                self.assertLess(abs(value - exact(t)), 1e-7)
            # The depleted depot may be reported slightly below zero; it is not clamped.
            self.assertTrue(all(a > -1e-10 for a in outputs["depot_amount"]))

    def test_supplied_negative_initial_state_is_still_rejected(self):
        model = qq.CompiledModel(MODEL)
        run = request("diffsol_bdf")
        run["initial_states"] = {"depot": qq.quantity(-1e-13, "mg"),
                                 "central": qq.quantity(0, "mg")}
        with self.assertRaises(qq.QSPQuestError):
            model.run(run)

    def test_bdf_fit_uses_the_same_invariant_check(self):
        model = qq.CompiledModel(MODEL)
        times = [1, 2, 4, 8, 24, 48, 72]
        run = request("diffsol_bdf", 72, times)
        observations = [
            {"row": i, "source_row": i, "output": "cp",
             "value": {"value": exact(t), "unit": "mg/L"}}
            for i, t in enumerate(times)
        ]
        fit = model.fit_data(
            run, observations,
            {"cl": {"initial": 2.0, "lower": 0.5, "upper": 10.0, "unit": "L/h"}},
            {"model": "normal", "additive": 0.01, "proportional": 0.0},
            method="derivative_free", max_iterations=200, max_simulations=2000)
        self.assertEqual(fit["status"], "converged")
        self.assertLess(abs(fit["parameters"]["cl"]["value"] - 2.4), 1e-3)

    def test_forward_sensitivities_complete_after_the_depot_is_depleted(self):
        sensitive = qq.CompiledSensitivities(MODEL, ["cl"])
        result = sensitive.run({
            "schema": "qspquest.sensitivity/v0.1",
            "run": request("diffsol_bdf"),
            "with_respect_to": ["cl"],
            "derivative_absolute_tolerances": {
                "cl": {"depot": qq.quantity(1e-10, "mg.h/L"),
                       "central": qq.quantity(1e-10, "mg.h/L")}},
        })
        self.assertTrue(any(c["parameter"] == "cl" for c in result["derivatives"]))


if __name__ == "__main__":
    unittest.main()
