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
    def test_locked_backend_reads_only_the_diffsol_package(self):
        revision = 'a' * 40
        package = ('[[package]]\nname = "diffsol"\nversion = "0.18.0"\n'
                   'source = "git+https://github.com/martinjrobins/diffsol?rev='
                   + revision + '#' + revision + '"\n')
        lock = ('version = 4\n[[package]]\nname = "application"\nversion = "1.0.0"\n'
                'dependencies = ["diffsol"]\n' + package +
                '[[package]]\nname = "diffsol-la"\nversion = "0.3.0"\n')
        self.assertEqual(module.locked_backend_version(lock), '0.18.0+git:' + revision)

    def test_locked_backend_rejects_missing_duplicate_or_malformed_identity(self):
        revision = 'a' * 40
        package = ('[[package]]\nname = "diffsol"\nversion = "0.18.0"\n'
                   'source = "git+https://github.com/martinjrobins/diffsol?rev='
                   + revision + '#' + revision + '"\n')
        malformed = {
            'missing package': 'version = 4\n',
            'duplicate package': package + package,
            'missing source': '\n'.join(line for line in package.splitlines() if not line.startswith('source =')),
            'unquoted source': package.replace('source = "git+', 'source = git+'),
            'duplicate source': package + 'source = "invalid"\n',
            'malformed duplicate source': package + 'source = invalid\n',
            'duplicate name': package + 'name = "another-package"\n',
            'missing version': package.replace('version = "0.18.0"\n', ''),
            'invalid version': package.replace('0.18.0', 'not-a-version'),
            'registry source': package.replace('git+https://github.com/martinjrobins/diffsol?rev=' + revision + '#' + revision,
                                               'registry+https://github.com/rust-lang/crates.io-index'),
            'other repository': package.replace('martinjrobins/diffsol', 'example/another-solver'),
            'short revision': package.replace(revision, 'abcdef'),
            'different resolved revision': package.replace('#' + revision, '#' + 'b' * 40),
        }
        for case, lock in malformed.items():
            with self.subTest(case=case), self.assertRaises(ValueError):
                module.locked_backend_version(lock)

    def test_changed_source_and_wrong_artifact_identity_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in module.FIXED:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'synthetic source fixture\n')
            for name in [*(directory + '/lib.rs' for directory in module.DIRECTORIES)]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'synthetic source fixture\n')
            (root / 'Cargo.lock').write_text(
                '[[package]]\nname = "diffsol"\nversion = "0.18.0"\n'
                'source = "git+https://github.com/martinjrobins/diffsol?rev=' + 'a' * 40 + '#' + 'a' * 40 + '"\n')
            source, backend = module.source_hashes(root)
            identity = {'compiler_source_sha256': source, 'backend_version': backend, 'target': 'fixture-target'}
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
