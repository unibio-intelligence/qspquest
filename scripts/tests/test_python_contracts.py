"""Schema, runtime signature and usable result type regressions."""
import ast
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from check_python_contracts import check
from generate_python_types import generate


class PythonContractTests(unittest.TestCase):
    def test_all_native_contracts_and_public_signatures_match(self):
        report = check(ROOT)
        self.assertEqual(report['status'], 'matched')
        self.assertEqual(report['generated_native_schemas'], 20)
        self.assertEqual(report['result_methods'], 10)
        self.assertGreaterEqual(report['signatures'], 61)

    def copy_source(self, root):
        package = Path('bindings/python/python/qspquest')
        shutil.copytree(ROOT / package, root / package, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        shutil.copytree(ROOT / 'spec/schema', root / 'spec/schema')
        return root / package

    def test_plain_dictionary_return_regression_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = self.copy_source(root)
            path = package / '__init__.pyi'
            text = path.read_text().replace('-> AnalysisResult:', '-> dict[str, Any]:', 1)
            path.write_text(text)
            with self.assertRaisesRegex(ValueError, 'missing its AnalysisResult type'):
                check(root)

    def test_missing_forwarded_option_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = self.copy_source(root)
            path = package / '__init__.pyi'
            path.write_text(path.read_text().replace('max_simulations: int = ...', 'max_simulation: int = ...', 1))
            with self.assertRaisesRegex(ValueError, 'Keyword signature differs'):
                check(root)

    def test_new_solver_schema_cannot_leave_types_stale(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.copy_source(root)
            path = root / 'spec/schema/run.schema.json'
            schema = json.loads(path.read_text())
            schema['definitions']['Solver']['enum'].append('synthetic_new_solver')
            path.write_text(json.dumps(schema))
            with self.assertRaisesRegex(ValueError, 'differ from the current native schemas'):
                check(root)

    def test_dataset_field_drift_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = self.copy_source(root)
            path = package / '__init__.pyi'
            path.write_text(path.read_text().replace('subject_ids: tuple[str, ...]', 'subject_keys: tuple[str, ...]'))
            with self.assertRaisesRegex(ValueError, 'dataset fields or dataclass constructor differ'):
                check(root)

    def test_required_keys_and_dictionary_interoperability(self):
        path = ROOT / 'bindings/python/python/qspquest/types.py'
        name = 'native_schema_contract_test'
        spec = importlib.util.spec_from_file_location(name, path)
        types = importlib.util.module_from_spec(spec)
        sys.modules[name] = types
        try:
            spec.loader.exec_module(types)
            self.assertEqual(types.RunRequest.__required_keys__, {'schema', 'solver', 'budgets', 'regimen'})
            self.assertIn('request_id', types.RunRequest.__optional_keys__)
            value = types.Quantity(value=300.0, unit='mg')
            self.assertIs(type(value), dict)
            self.assertEqual(value, {'value': 300.0, 'unit': 'mg'})
        finally:
            sys.modules.pop(name, None)

    def test_python_310_grammar_and_deterministic_generation(self):
        text = generate(ROOT)
        ast.parse(text, feature_version=(3, 10))
        self.assertEqual(text, (ROOT / 'bindings/python/python/qspquest/types.py').read_text())


if __name__ == '__main__':
    unittest.main()
