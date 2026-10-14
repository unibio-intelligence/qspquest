"""The packaged first-run check must reject malformed successful responses."""
from copy import deepcopy
import importlib.util
import math
from pathlib import Path
import unittest


class McpQuickstartTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).resolve().parents[2] / 'bindings/mcp/scripts/quickstart.py'
        spec = importlib.util.spec_from_file_location('mcp_quickstart', path)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def setUp(self):
        self.result = {
            'schema': 'qspquest.result/v0.1', 'model_id': 'synthetic_one_compartment',
            'contract': {'status': 'success'}, 'times': [0, 2, 8, 24],
            'time_unit': 'h', 'sides': ['post'] * 4,
            'outputs': [{'name': 'cp', 'unit': 'mg/L',
                         'values': [2 * math.exp(-.1 * t) for t in [0, 2, 8, 24]]}]}

    def test_complete_result_passes(self):
        column = self.module.check_simulation(self.result)
        self.assertEqual(column, self.result['outputs'][0])

    def test_duplicate_missing_extra_or_wrong_output_fails(self):
        changes = [lambda r: r.update(outputs=[]),
                   lambda r: r.update(outputs=None),
                   lambda r: r.update(outputs=[None]),
                   lambda r: r['outputs'].append(deepcopy(r['outputs'][0])),
                   lambda r: r['outputs'].append({'name': 'cp', 'unit': 'g/L', 'values': [-999] * 4}),
                   lambda r: r['outputs'].append({'name': 'unrequested', 'unit': 'mg/L', 'values': [2] * 4}),
                   lambda r: r['outputs'][0].update(name='other'),
                   lambda r: r['outputs'][0].update(unit='g/L')]
        self.check_invalid(changes)

    def test_identity_status_and_observation_coordinates_must_match(self):
        changes = [lambda r: r.update(schema='other'),
                   lambda r: r.update(model_id='other'),
                   lambda r: r.update(contract={'status': 'scientific_failure'}),
                   lambda r: r.update(error={'code': 'solver'}),
                   lambda r: r.update(time_unit='d'),
                   lambda r: r.update(times=[False, 2, 8, 24]),
                   lambda r: r.update(times=[0, 2, 8, float('inf')]),
                   lambda r: r.update(times=[0, 2, 8, float('nan')]),
                   lambda r: r.update(times=[0, 2, 8]),
                   lambda r: r.update(times=[0, 2, 8, 25]),
                   lambda r: r.update(times=[]),
                   lambda r: r.update(sides=['post'] * 3),
                   lambda r: r.update(sides=['pre', 'post', 'post', 'post'])]
        self.check_invalid(changes)

    def test_concentrations_must_be_complete_finite_numbers_and_accurate(self):
        for values in ([], [2], [2] * 5, [True] * 4, [None] * 4,
                       ['2'] * 4, [float('nan')] * 4, [float('inf')] * 4, [-999] * 4):
            with self.subTest(values=values):
                changed = deepcopy(self.result)
                changed['outputs'][0]['values'] = values
                with self.assertRaises(ValueError):
                    self.module.check_simulation(changed)

    def check_invalid(self, changes):
        for change in changes:
            result = deepcopy(self.result)
            change(result)
            with self.subTest(result=result), self.assertRaises(ValueError):
                self.module.check_simulation(result)


if __name__ == '__main__':
    unittest.main()
