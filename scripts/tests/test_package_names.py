"""Validate current distribution and document identities."""
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_package_names import PACKAGES, check

ROOT = Path(__file__).resolve().parents[2]


class DistributionNames(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.root = Path(self.folder.name)
        files = list(PACKAGES)+['Cargo.lock','pyproject.toml','README.md','bindings/r/DESCRIPTION',
            'bindings/r/NAMESPACE','bindings/wasm/package.json','bindings/python/python/qspquest/__init__.py']
        for path in files:
            target = self.root/path
            target.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(ROOT/path,target)

    def test_current_names(self):
        self.assertEqual(check(self.root), [])

    def test_document_schema_namespace_is_explicit(self):
        path = self.root/'spec/schema/model.json'
        path.parent.mkdir(parents=True)
        for schema in ['other.model/v0.1', 'qspquest.model', None]:
            import json
            path.write_text(json.dumps({'properties': {'schema': {'enum': [schema]}}}))
            self.assertTrue(check(self.root), schema)
        path.write_text('{"properties":{"schema":{"enum":["qspquest.model/v0.1"]}}}')
        self.assertEqual(check(self.root), [])

    def test_wrong_engine_package_and_python_distribution_fail(self):
        for path,old,new in [('crates/qspquest-modeling/Cargo.toml','qspquest-modeling','other-modeling'),
                             ('pyproject.toml','name = "qspquest"','name = "other-engine"')]:
            file = self.root/path
            saved = file.read_text()
            file.write_text(saved.replace(old,new,1))
            self.assertTrue(check(self.root))
            file.write_text(saved)

    def test_extra_python_package_fails(self):
        path = self.root/'bindings/python/python/other_engine/__init__.py'
        path.parent.mkdir(parents=True)
        path.write_text('')
        self.assertTrue(check(self.root))


if __name__ == '__main__':
    unittest.main()
