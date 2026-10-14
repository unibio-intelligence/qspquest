"""Reject altered source and installed-artifact identities before qualification."""
from pathlib import Path
import importlib.util
import json
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('check_source_identity', Path(__file__).resolve().parents[1] / 'check_source_identity.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SourceIdentityTests(unittest.TestCase):
    def test_changed_source_and_wrong_artifact_identity_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in module.FIXED:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'synthetic source fixture\n')
            for name in [*(directory + '/lib.rs' for directory in module.DIRECTORIES), 'vendor/diffsol/src/lib.rs']:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'synthetic source fixture\n')
            source, vendor = module.source_hashes(root)
            identity = {'compiler_source_sha256': source, 'backend_version': 'fixture+sha256:' + vendor, 'target': 'fixture-target'}
            report = {'schema': 'qspquest.doctor/v0.1', 'status': 'passed', 'checks': [
                {'name': 'analytic_simulation', 'status': 'passed', 'detail': {'identity': identity}}]}
            self.assertEqual(module.verify(root, report, 'fixture-target')['status'], 'verified')
            for field in ('compiler_source_sha256', 'target', 'backend_version'):
                changed = json.loads(json.dumps(report))
                changed['checks'][0]['detail']['identity'][field] = 'altered'
                with self.subTest(field=field), self.assertRaises(ValueError):
                    module.verify(root, changed, 'fixture-target')
            shared = root / 'bindings/mcp/src/core/compute.rs'
            original = shared.read_bytes()
            shared.write_bytes(b'changed raw WASM dispatch fixture\n')
            with self.assertRaises(ValueError):
                module.verify(root, report, 'fixture-target')
            shared.write_bytes(original)
            (root / module.DIRECTORIES[0] / 'lib.rs').write_bytes(b'changed source fixture\n')
            with self.assertRaises(ValueError):
                module.verify(root, report, 'fixture-target')


if __name__ == '__main__':
    unittest.main()
