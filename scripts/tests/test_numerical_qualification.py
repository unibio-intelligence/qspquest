"""Shared malformed results must never establish cross-target agreement."""
import copy
import importlib.util
import hashlib
import json
import zipfile
from pathlib import Path
import sys
import unittest

try:
    import numpy
    import scipy
except ImportError:
    HAS_SCIENTIFIC_DEPS = False
else:
    HAS_SCIENTIFIC_DEPS = True


class BundledPolicyTests(unittest.TestCase):
    def test_every_policy_input_matches_current_archive(self):
        root = Path(__file__).resolve().parents[2]
        library = root / 'bindings/mcp/assets/library'
        policy = json.loads((root / 'bindings/mcp/qualification/output-bounds.json').read_text())
        rules = {row['case']: row for row in policy['cases']}
        self.assertEqual(len(rules), len(policy['cases']))
        self.assertEqual(len(rules), policy['expected_scenarios'])
        seen = set()
        def digest(value):
            return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()).hexdigest()
        for metadata in json.loads((library / 'catalog.json').read_text())['models']:
            with zipfile.ZipFile(library / metadata['archive']) as archive:
                index = json.loads(archive.read('index.json'))
                model = json.loads(archive.read(index['native_execution']['model']))
                for scenario in metadata['scenario_ids']:
                    case = metadata['id'] + '--' + scenario
                    self.assertNotIn(case, seen)
                    seen.add(case)
                    request = json.loads(archive.read('native/scenarios/' + scenario + '.json'))
                    with self.subTest(case=case):
                        self.assertEqual(rules[case]['input_model_sha256'], digest(model))
                        self.assertEqual(rules[case]['input_request_sha256'], digest(request))
        self.assertEqual(seen, set(rules))


@unittest.skipUnless(HAS_SCIENTIFIC_DEPS, 'Numerical qualification requires NumPy and SciPy')
class NumericalQualificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        folder = Path(__file__).resolve().parents[2] / 'bindings/mcp/scripts'
        sys.path.insert(0, str(folder))
        try:
            spec = importlib.util.spec_from_file_location('qualification', folder / 'check_numerical_qualification.py')
            cls.module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.module)
        finally:
            sys.path.pop(0)

    def setUp(self):
        self.data = {'model': {'time_unit': 'h'}, 'request': {'regimen': {'observations': [
            {'time': {'value': 0, 'unit': 'h'}, 'side': 'pre'},
            {'time': {'value': 2, 'unit': 'h'}, 'side': 'post'}]}}}
        result = {'times': [0, 2], 'sides': ['pre', 'post'], 'time_unit': 'h',
                  'outputs': [{'name': 'cp', 'unit': 'mg/L', 'values': [0., 1.]}],
                  'identity': {'model_content_hash': 'model', 'bound_value_hash': 'bound', 'run_request_hash': 'request'}}
        self.data.update(result=copy.deepcopy(result), wasm_result=copy.deepcopy(result))
        self.rule = {'mode': 'legacy_absolute_regression', 'model_content_hash': 'model',
                     'run_request_hash': 'request', 'outputs': [{'name': 'cp', 'unit': 'mg/L', 'absolute': 1e-9, 'relative': 0}],
                     'input_model_sha256': self.module.input_hash(self.data['model']),
                     'input_request_sha256': self.module.input_hash(self.data['request'])}

    def check(self, data):
        return self.module.check_case(data, self.rule, Path('/unused'), 'synthetic')

    def test_complete_matching_results_and_numerical_mismatch(self):
        self.assertTrue(self.check(self.data)['qualified'])
        self.data['wasm_result']['outputs'][0]['values'][1] += .1
        self.assertFalse(self.check(self.data)['qualified'])

    def test_identical_truncated_or_invalid_targets_cannot_pass(self):
        mutations = [lambda r: r['outputs'][0].update(values=[]),
                     lambda r: r['outputs'][0].update(values=[0.]),
                     lambda r: r['outputs'][0].update(values=[0., 1., 2.]),
                     lambda r: r['outputs'][0].update(values=[False, 1.]),
                     lambda r: r['outputs'][0].update(values=[float('nan'), 1.]),
                     lambda r: r['outputs'][0].update(values=[float('inf'), 1.]),
                     lambda r: r.update(times=[100, 102]),
                     lambda r: r.update(times=[False, 2]),
                     lambda r: r.update(sides=['post', 'post']),
                     lambda r: r.update(time_unit='day'),
                     lambda r: r.update(outputs=[]),
                     lambda r: r['outputs'].append(copy.deepcopy(r['outputs'][0]))]
        for index, mutation in enumerate(mutations):
            data = copy.deepcopy(self.data)
            for target in ('result', 'wasm_result'):
                mutation(data[target])
            with self.subTest(mutation=index), self.assertRaises(ValueError):
                self.check(data)


if __name__ == '__main__':
    unittest.main()
