"""Population dataset conversion keeps source and scientific row semantics explicit."""

import copy
import json
import math
from pathlib import Path
import unittest

import qspquest


ROOT = Path(__file__).resolve().parents[3]


def template(kind="focei"):
    sensitivity = json.loads(
        (ROOT / "conformance/requests/synthetic-sensitivity.json").read_text()
    )
    sensitivity["with_respect_to"] = ["cl"]
    sensitivity["derivative_absolute_tolerances"].pop("v")
    run = sensitivity["run"]
    run["parameters"] = {
        "cl": {"value": 0.5, "unit": "L/h"},
        "v": {"value": 4.5, "unit": "L"},
    }
    run["regimen"]["administrations"] = []
    run["regimen"]["samples"] = []
    body = {
        "subjects": [{"simulation": sensitivity, "observations": [], "priors": []}],
        "fixed_effects": [
            {
                "parameter": "cl",
                "lower": {"value": 0.35, "unit": "L/h"},
                "upper": {"value": 0.85, "unit": "L/h"},
            }
        ],
        "random_effects": [{"parameter": "cl"}],
        "omega": [[0.04]],
        "omega_diagonal_bounds": [[0.04, 0.04]],
        "eta_bound": 1.5,
        "max_evaluations": 30000,
        "max_iterations": 80,
        "gradient_tolerance": 0.005,
        "seed": 17,
    }
    return {"schema": "qspquest.fit/v0.1", "problem": {"kind": kind, "request": body}}


ENDPOINTS = {
    2: {
        "output": "cp",
        "value_unit": "mg/L",
        "error": {
            "additive_sd": {"value": 0.05, "unit": "mg/L"},
            "proportional_sd": 0.0,
        },
    }
}


def prepare(rows, kind="focei", **kwargs):
    return qspquest.population_fit_dataset(
        rows,
        template(kind),
        compartments={1: "central"},
        endpoints=ENDPOINTS,
        time_unit="h",
        amount_unit="mg",
        **kwargs,
    )


class PopulationDatasetTests(unittest.TestCase):
    def test_template_requires_current_schemas_without_mutating_input(self):
        rows = [{"ID": "A", "TIME": 1, "EVID": 0, "CMT": 2, "DV": 1.2}]
        current = template()
        before = copy.deepcopy(current)
        def convert(value):
            return qspquest.population_fit_dataset(
                rows, value, compartments={1: "central"}, endpoints=ENDPOINTS,
                time_unit="h", amount_unit="mg").request
        self.assertEqual(convert(current)["schema"], "qspquest.fit/v0.1")
        self.assertEqual(current, before)
        simulation = current["problem"]["request"]["subjects"][0]["simulation"]
        for document in (current, simulation, simulation["run"]):
            tag = document["schema"]
            for invalid in (tag.replace("qspquest.", "other."), tag.replace("v0.1", "v99"), None, [tag], 1):
                document["schema"] = invalid
                rejected_input = copy.deepcopy(current)
                with self.assertRaises(qspquest.QSPQuestError):
                    convert(current)
                self.assertEqual(current, rejected_input)
            del document["schema"]
            with self.assertRaises(qspquest.QSPQuestError):
                convert(current)
            document["schema"] = tag

    def test_request_preserves_subject_order_source_rows_and_observation_side(self):
        rows = [
            {"ID": "B", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6},
            {"ID": "A", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6},
            {"ID": "B", "TIME": 0, "EVID": 0, "CMT": 2, "DV": 0},
            {"ID": "B", "TIME": 1, "EVID": 0, "CMT": 2, "DV": 1.1},
            {"ID": "A", "TIME": 1, "EVID": 0, "CMT": 2, "DV": 1.2},
            {"ID": "B", "TIME": 2, "EVID": 0, "MDV": 1, "DV": float("nan")},
        ]
        before = copy.deepcopy(rows)
        fit_template = template()
        original_template = copy.deepcopy(fit_template)
        dataset = qspquest.population_fit_dataset(
            rows,
            fit_template,
            compartments={1: "central"},
            endpoints=ENDPOINTS,
            time_unit="h",
            amount_unit="mg",
            observation_side="pre",
        )
        self.assertEqual(rows, before)
        self.assertEqual(fit_template, original_template)
        self.assertEqual(dataset.subject_ids, ("B", "A"))
        self.assertEqual(
            [
                (row["subject_id"], row["source_row"], row["result_row"])
                for row in dataset.mapping
            ],
            [("B", 2, 0), ("B", 3, 1), ("A", 4, 0)],
        )
        subjects = dataset.request["problem"]["request"]["subjects"]
        self.assertEqual(
            subjects[0]["simulation"]["run"]["regimen"]["observations"][0]["side"],
            "pre",
        )
        self.assertEqual([o["row"] for o in subjects[0]["observations"]], [0, 1])
        self.assertEqual(
            subjects[0]["simulation"]["run"]["regimen"]["administrations"][0]["target"],
            "central",
        )

    def test_invalid_data_reports_structured_errors(self):
        dose = {"ID": "A", "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6}
        observation = {"ID": "A", "TIME": 1, "EVID": 0, "CMT": 2, "DV": 1}
        cases = [
            ([{**dose, "SS": 1}, observation], {}, "SS"),
            ([dose, {**observation, "DV": float("nan")}], {}, "DV"),
            ([dose, {**observation, "CMT": 3}], {}, "endpoint mapping"),
            ([{**dose, "RATE": 2}, observation], {}, "rate_unit"),
            ([dose, {**observation, "WT": 70}], {}, "unsupported columns"),
            (
                [dose, observation],
                {"observation_side": "dose_first"},
                "observation_side",
            ),
        ]
        for rows, options, message in cases:
            with self.subTest(message=message), self.assertRaises(
                qspquest.QSPQuestError
            ) as caught:
                prepare(rows, **options)
            self.assertEqual(caught.exception.code, "invalid_input")
            self.assertIn(message, str(caught.exception))

    def test_dataframe_protocol_needs_no_pandas_dependency(self):
        rows = [
            {"ID": 1, "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6},
            {"ID": 1, "TIME": 1, "EVID": 0, "CMT": 2, "DV": 1.1},
        ]

        class Frame:
            columns = ("ID", "TIME", "EVID", "CMT", "AMT", "DV")

            def to_dict(self, orient):
                assert orient == "records"
                return rows

        dataset = prepare(Frame())
        self.assertEqual(dataset.subject_ids, ("1",))
        self.assertEqual(dataset.mapping[0]["source_row"], 1)
        try:
            import pandas as pd
            import numpy as np
        except ImportError:
            return
        # pandas converts NumPy column scalars to Python numbers in records.
        frame = pd.DataFrame(
            [
                {"ID": 1, "TIME": 0.0, "EVID": 1, "CMT": 1, "AMT": 6.0, "DV": 0.0},
                {"ID": 1, "TIME": 1.0, "EVID": 0, "CMT": 2, "AMT": 0.0, "DV": 1.1},
            ]
        )
        self.assertEqual(prepare(frame).mapping[0]["source_row"], 1)
        numpy_rows = [
            {
                "ID": np.int64(1),
                "TIME": np.float32(0),
                "EVID": np.int64(1),
                "CMT": np.int64(1),
                "AMT": np.float64(6),
            },
            {
                "ID": np.int64(1),
                "TIME": np.float32(1),
                "EVID": np.int64(0),
                "CMT": np.int64(2),
                "DV": np.float64(1.1),
            },
        ]
        self.assertEqual(prepare(numpy_rows).subject_ids, ("1",))

    def test_focei_fit_returns_mapped_predictions_and_saem_accepts_same_rows(self):
        model = json.loads(
            (ROOT / "conformance/models/synthetic-one-compartment.json").read_text()
        )
        compiled = qspquest.CompiledSensitivities(model, ["cl"])
        rows = []
        for index, eta in enumerate((-0.22, -0.08, 0.08, 0.22)):
            identifier = f"S{index + 1}"
            rows.append({"ID": identifier, "TIME": 0, "EVID": 1, "CMT": 1, "AMT": 6})
            for time in (1, 2, 4):
                dv = 6 / 4.5 * math.exp(-0.6 * math.exp(eta) * time / 4.5)
                rows.append(
                    {"ID": identifier, "TIME": time, "EVID": 0, "CMT": 2, "DV": dv}
                )
        dataset = prepare(rows)
        fitted = compiled.fit_population_data(dataset)
        self.assertEqual(fitted["result"]["population"]["status"], "converged")
        self.assertEqual(len(fitted["fitted_observations"]), 12)
        for item in fitted["fitted_observations"]:
            self.assertEqual(
                item["subject_id"], fitted["subject_ids"][item["subject_index"]]
            )
            self.assertEqual(
                item["source_row"],
                fitted["mapping"][
                    item["subject_index"] * 3 + item["observation_index"]
                ]["source_row"],
            )
            source = rows[item["source_row"]]
            self.assertEqual(item["time"]["value"], source["TIME"])
            self.assertEqual(item["side"], "post")
            self.assertAlmostEqual(item["value"]["value"], source["DV"])
        saem = prepare(rows, kind="saem")
        saem.request["problem"]["request"]["max_iterations"] = 8
        saem_result = compiled.fit_population_data(saem)
        self.assertEqual(
            saem_result["result"]["population"]["objective_kind"],
            "saem_marginal_objective_function_value",
        )
        self.assertTrue(math.isfinite(saem_result["result"]["population"]["objective"]))
        self.assertEqual(len(saem_result["fitted_observations"]), 12)
        for item in saem_result["fitted_observations"]:
            source = rows[item["source_row"]]
            self.assertEqual(item["subject_id"], source["ID"])
            self.assertEqual(item["time"]["value"], source["TIME"])
            self.assertAlmostEqual(item["value"]["value"], source["DV"])
            self.assertTrue(math.isfinite(item["pred"]["value"]))
            self.assertTrue(math.isfinite(item["ipred"]["value"]))

        saem.request["problem"]["request"]["burn_in_iterations"] = 4
        saem.request["problem"]["request"]["additive_error_fit"] = {
            "output": "cp",
            "initial": {"value": 0.05, "unit": "mg/L"},
            "lower": {"value": 0.01, "unit": "mg/L"},
            "upper": {"value": 0.1, "unit": "mg/L"},
        }
        free_error = compiled.fit_population_data(saem)["result"]["population"]
        self.assertEqual(free_error["saem_diagnostics"]["burn_in_iterations"], 4)
        self.assertGreater(
            free_error["estimated_residual_error"]["additive_sd"]["value"], 0
        )


if __name__ == "__main__":
    unittest.main()
