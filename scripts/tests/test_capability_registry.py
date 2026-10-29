"""Reject stale or contradictory interface and scientific capability metadata."""
from copy import deepcopy
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from capability_registry import check, expand, validate
from check_source_identity import DIRECTORIES, FIXED


class CapabilityRegistryTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads((ROOT / 'spec/capabilities.json').read_text())

    def source(self, directory):
        root = Path(directory)
        paths = {'Cargo.toml', 'spec/capabilities.json', 'spec/schema/capability-catalog.schema.json',
                 'docs/site/capabilities.md', *self.catalog['source_snapshot']}
        paths.update(path for example in self.catalog['examples'] for path in example['paths'])
        paths.update(str(p.relative_to(ROOT)) for directory in DIRECTORIES for p in (ROOT / directory).rglob('*.rs'))
        paths.update(FIXED)
        paths.update(str(p.relative_to(ROOT)) for p in (ROOT / 'vendor/diffsol').rglob('*')
                     if p.is_file() and p.name != 'Cargo.lock'
                     and not {'target', '.git'}.intersection(p.relative_to(ROOT).parts))
        for name in paths:
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        return root

    def entry(self, interface, operation):
        return next(e for e in self.catalog['entries'] if e['interface'] == interface and e['operation'] == operation)

    def test_current_source_and_matrix_match(self):
        report = check(ROOT)
        self.assertEqual(report['status'], 'matched')
        self.assertGreater(report['tuples'], 1000)
        self.assertEqual(report['scientific_qualification'], 'not_assessed')

    def test_interface_exposure_and_estimators_are_distinct(self):
        rows = expand(validate(ROOT))
        self.assertFalse(any(r['operation'] == 'sensitivities' and r['interface'] == 'r' for r in rows))
        self.assertTrue(any(r['operation'] == 'fit_scalar' and r['interface'] == 'cli' and r['estimator'] == 'scalar_gaussian' for r in rows))
        self.assertEqual(self.entry('python_analysis', 'analysis_fit_population')['estimators'], ['saem_python'])
        self.assertIn('saem_native', self.entry('python', 'fit_population')['estimators'])
        self.assertEqual(self.entry('python_analysis', 'profile_likelihood')['estimators'], ['L-BFGS-B'])
        self.assertEqual({e['operation'] for e in self.catalog['entries'] if e['interface'] == 'wasm_worker'}, {'simulate'})

    def test_discrete_commands_and_profiled_fitting_are_registered(self):
        for operation in ('fit_gamma_poisson', 'fit_probit_normal_binomial'):
            for interface in ('rust', 'python', 'r', 'cli'):
                entry = self.entry(interface, operation)
                self.assertTrue(entry['implemented'] and entry['exposed'])
                self.assertEqual(entry['solvers'], ['none'])
            self.assertEqual(self.entry('cli', operation)['api'], 'qspquest fit-data')
        for interface in ('rust', 'python', 'r', 'cli', 'mcp'):
            self.assertIn('profiled_proportional', self.entry(interface, 'fit_individual')['estimators'])

    def test_changed_source_is_not_silently_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            p = root / 'bindings/mcp/src/policy.rs'
            p.write_text(p.read_text() + '\n// Changed admission policy.\n')
            with self.assertRaisesRegex(ValueError, 'snapshot is stale'):
                validate(root)

    def test_absent_symbol_is_refused_even_with_unchanged_source(self):
        self.entry('python', 'simulate')['symbol'] = 'def a_method_that_does_not_exist('
        with self.assertRaisesRegex(ValueError, 'symbol is absent'):
            validate(ROOT, self.catalog)

    def test_unknown_keys_and_integer_booleans_are_refused(self):
        for mutation in ('unknown', 'boolean'):
            catalog = deepcopy(self.catalog)
            if mutation == 'unknown':
                catalog['entries'][0]['qualification'] = True
            else:
                catalog['entries'][0]['implemented'] = 1
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate(ROOT, catalog)

    def test_duplicate_expanded_tuple_is_refused(self):
        self.catalog['entries'].append(deepcopy(self.catalog['entries'][0]))
        with self.assertRaisesRegex(ValueError, 'Duplicate expanded'):
            validate(ROOT, self.catalog)

    def test_catalog_cannot_promote_source_declarations_to_qualification(self):
        self.catalog['entries'][0]['evidence_status'] = 'passed'
        self.catalog['entries'][0]['evidence_ids'] = ['an_unbound_claim']
        with self.assertRaises(ValueError):
            validate(ROOT, self.catalog)

    def test_stale_compiler_identity_is_refused(self):
        self.catalog['compiler_source_sha256'] = 'sha256:' + '0' * 64
        with self.assertRaisesRegex(ValueError, 'source identity is stale'):
            validate(ROOT, self.catalog)

    def test_missing_example_and_unknown_example_id_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            (root / 'docs/examples/python_typed.py').unlink()
            with self.assertRaisesRegex(ValueError, 'absent or unsafe'):
                validate(root)
        self.catalog['entries'][0]['example_ids'].append('not-a-delivered-example')
        with self.assertRaisesRegex(ValueError, 'absent example ID'):
            validate(ROOT, self.catalog)

    def test_feature_gates_cannot_disappear(self):
        for interface, operation in [('mcp', 'steady_state'), ('wasm_direct', 'sensitivities'), ('mcp', 'submit_simulation'), ('wasm_sandbox', 'simulate')]:
            catalog = deepcopy(self.catalog)
            next(e for e in catalog['entries'] if e['interface'] == interface and e['operation'] == operation)['required_features'] = []
            with self.subTest(interface=interface), self.assertRaises(ValueError):
                validate(ROOT, catalog)

    def test_missing_router_tool_is_refused(self):
        self.catalog['entries'] = [e for e in self.catalog['entries'] if e['api'] != 'get_capability']
        self.catalog['operation_labels'].pop('get_capability')
        with self.assertRaisesRegex(ValueError, 'tool router'):
            validate(ROOT, self.catalog)

    def test_withheld_mcp_solver_and_worker_operations_are_refused(self):
        self.entry('mcp', 'simulate')['solvers'].append('diffsol_rosenbrock23')
        with self.assertRaisesRegex(ValueError, 'withholds'):
            validate(ROOT, self.catalog)
        self.entry('mcp', 'simulate')['solvers'].remove('diffsol_rosenbrock23')
        self.entry('wasm_worker', 'simulate')['solvers'].append('diffsol_rodas5p')
        with self.assertRaisesRegex(ValueError, 'Worker contract'):
            validate(ROOT, self.catalog)

    def test_matrix_drift_and_package_version_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            (root / 'docs/site/capabilities.md').write_text('Everything is qualified.\n')
            with self.assertRaisesRegex(ValueError, 'matrix is stale'):
                check(root)
        self.catalog['package_version'] = '99.0.0'
        with self.assertRaisesRegex(ValueError, 'version is stale'):
            validate(ROOT, self.catalog)


if __name__ == '__main__':
    unittest.main()
