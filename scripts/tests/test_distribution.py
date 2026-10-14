"""Synthetic archive regressions; these fixtures never load a native extension."""
import csv
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
import warnings
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import check_distribution as distribution


def package_metadata():
    return ('Metadata-Version: 2.4\nName: qspquest\nVersion: 0.1.0\n'
            'Description-Content-Type: text/markdown; charset=UTF-8; variant=GFM\n\n' +
            (ROOT / 'bindings/python/README.md').read_text()).encode()


class DistributionTests(unittest.TestCase):
    def source_archive(self, directory, *, wrapper=True, omitted=None, changed=None,
                       extra=None, symlink=False, duplicate=False, directory_member=False):
        path = Path(directory) / 'qspquest-test.tar.gz'
        prefix = 'qspquest-test/' if wrapper else ''
        with tarfile.open(path, 'w:gz') as archive:
            if directory_member and wrapper:
                member = tarfile.TarInfo('qspquest-test')
                member.type = tarfile.DIRTYPE
                archive.addfile(member)
            for name, source in distribution.source_files(ROOT).items():
                if name == omitted:
                    continue
                raw = b'changed' if name == changed else source.read_bytes()
                member = tarfile.TarInfo(prefix + name)
                member.size = len(raw)
                archive.addfile(member, io.BytesIO(raw))
                if duplicate and name == 'Cargo.toml':
                    archive.addfile(member, io.BytesIO(raw))
            if omitted != 'PKG-INFO':
                raw = b'changed' if changed == 'PKG-INFO' else package_metadata()
                member = tarfile.TarInfo(prefix + 'PKG-INFO')
                member.size = len(raw)
                archive.addfile(member, io.BytesIO(raw))
            if extra:
                member = tarfile.TarInfo(prefix + extra)
                if symlink:
                    member.type = tarfile.SYMTYPE
                    member.linkname = '../../outside'
                    archive.addfile(member)
                else:
                    member.size = 1
                    archive.addfile(member, io.BytesIO(b'x'))
        return path

    def wheel(self, directory, *, omitted=None, changed=None, extra=None,
              record_change=None, duplicate=False, symlink=False):
        info = 'qspquest-0.1.0.dist-info'
        files = {name: source.read_bytes() for name, source in distribution.python_files(ROOT).items()}
        files.update({info + '/licenses/' + name: (ROOT / name).read_bytes()
                      for name in ('LICENSE', 'THIRD-PARTY-NOTICES.txt')})
        files['qspquest/_native.abi3.so'] = b'Synthetic fixture, never loaded.'
        files[info + '/METADATA'] = package_metadata()
        files[info + '/WHEEL'] = b'Wheel-Version: 1.0\nRoot-Is-Purelib: false\nTag: cp310-abi3-macosx_11_0_arm64\n'
        if omitted:
            files.pop(omitted)
        if changed:
            files[changed] = b'altered file with a matching RECORD'
        if extra:
            files[extra] = b'extra'
        record_name = info + '/RECORD'
        record = io.StringIO(newline='')
        writer = csv.writer(record)
        for name, raw in files.items():
            writer.writerow((name, 'sha256=' + distribution.digest(raw), str(len(raw))))
        writer.writerow((record_name, '', ''))
        raw_record = record.getvalue()
        if record_change:
            raw_record = record_change(raw_record)
        files[record_name] = raw_record.encode()
        path = Path(directory) / 'qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl'
        with zipfile.ZipFile(path, 'w') as archive:
            for name, raw in files.items():
                member = zipfile.ZipInfo(name)
                if symlink and name == extra:
                    member.create_system = 3
                    member.external_attr = 0o120777 << 16
                archive.writestr(member, raw)
                if duplicate and name == 'qspquest/control.py':
                    with warnings.catch_warnings():
                        warnings.simplefilter('ignore', UserWarning)
                        archive.writestr(member, raw)
        return path

    def test_source_accepts_wrapped_and_unwrapped_and_counts_wrapper(self):
        with tempfile.TemporaryDirectory() as directory:
            base = len(distribution.source_files(ROOT)) + 1
            for wrapper, directory_member in ((True, False), (False, False), (True, True)):
                with self.subTest(wrapper=wrapper, directory_member=directory_member):
                    path = self.source_archive(directory, wrapper=wrapper, directory_member=directory_member)
                    self.assertEqual(distribution.inspect_source(path), base + int(directory_member))

    def test_source_rejects_missing_build_or_python_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ('Cargo.lock', 'crates/qspquest-modeling/build.rs',
                         'bindings/r/src/rust/Cargo.toml', 'bindings/r/src/rust/src/lib.rs',
                         'bindings/python/python/qspquest/adapters/_bridge.py',
                         'bindings/python/README.md', 'SECURITY.md', 'PKG-INFO'):
                with self.subTest(name=name):
                    path = self.source_archive(directory, omitted=name)
                    with self.assertRaisesRegex(ValueError, 'incomplete source archive'):
                        distribution.inspect_source(path)

    def test_source_rejects_changed_legal_or_source_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ('THIRD-PARTY-NOTICES.txt', 'Cargo.lock', 'bindings/python/python/qspquest/control.py'):
                with self.subTest(name=name):
                    path = self.source_archive(directory, changed=name)
                    with self.assertRaisesRegex(ValueError, 'differs from selected source'):
                        distribution.inspect_source(path)

    def test_source_requires_matching_package_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.source_archive(directory, changed='PKG-INFO')
            with self.assertRaises(ValueError):
                distribution.inspect_source(path)

    def test_source_rejects_generated_and_unselected_material(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ('vendor/diffsol/target/debug/cache', 'bindings/python/__pycache__/a.pyc',
                         'crates/.DS_Store', 'internal/private.json',
                         'bindings/python/private.json', 'conformance/evidence/result.json'):
                with self.subTest(name=name):
                    path = self.source_archive(directory, extra=name)
                    with self.assertRaises(ValueError):
                        distribution.inspect_source(path)

    def test_source_rejects_duplicate_traversal_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            for kwargs in ({'duplicate': True}, {'extra': '../../outside'},
                           {'extra': 'vendor/link', 'symlink': True}):
                with self.subTest(kwargs=kwargs):
                    path = self.source_archive(directory, **kwargs)
                    with self.assertRaises(ValueError):
                        distribution.inspect_source(path)

    def test_wheel_accepts_complete_synthetic_package(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.wheel(directory)
            count = distribution.inspect_wheel(path)
            with zipfile.ZipFile(path) as archive:
                self.assertEqual(count, len(archive.infolist()))
            report = distribution.inspect_directory(Path(directory))
            self.assertEqual(report['schema'], 'qspquest.installation-artifacts/v0.1')
            self.assertEqual(report['artifacts'][0]['files'], count)

    def test_wheel_requires_every_python_module_policy_and_native(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ('qspquest/adapters/_bridge.py', 'qspquest/individual.py',
                         'qspquest/SECURITY.md', 'qspquest/py.typed', 'qspquest/_native.abi3.so'):
                with self.subTest(name=name):
                    path = self.wheel(directory, omitted=name)
                    with self.assertRaises(ValueError):
                        distribution.inspect_wheel(path)

    def test_wheel_rejects_altered_source_even_with_consistent_record(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ('qspquest/control.py', 'qspquest-0.1.0.dist-info/licenses/LICENSE'):
                with self.subTest(name=name):
                    path = self.wheel(directory, changed=name)
                    with self.assertRaisesRegex(ValueError, 'differs from selected source'):
                        distribution.inspect_wheel(path)

    def test_wheel_rejects_unrecorded_duplicate_and_bad_hash_or_size(self):
        with tempfile.TemporaryDirectory() as directory:
            transforms = (
                lambda value: value.replace('qspquest/control.py', 'qspquest/missing.py', 1),
                lambda value: value.replace('sha256=', 'sha256=invalid', 1),
                lambda value: value + value.splitlines()[0] + '\n',
                lambda value: value.replace(',\r\n', ',123\r\n'),
            )
            for transform in transforms:
                path = self.wheel(directory, record_change=transform)
                with self.assertRaises(ValueError):
                    distribution.inspect_wheel(path)
            with self.assertRaisesRegex(ValueError, 'duplicate archive member'):
                distribution.inspect_wheel(self.wheel(directory, duplicate=True))

    def test_wheel_rejects_unexpected_symlinked_or_signed_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            for kwargs in ({'extra': 'qspquest/private.json'}, {'extra': '../outside'},
                           {'extra': 'qspquest/link.py', 'symlink': True},
                           {'extra': 'qspquest/_native.pyd'},
                           {'extra': 'qspquest.libs/private.json'},
                           {'extra': 'qspquest-0.1.0.dist-info/private.json'},
                           {'extra': 'qspquest-0.1.0.dist-info/RECORD.jws'}):
                with self.subTest(kwargs=kwargs):
                    with self.assertRaises(ValueError):
                        distribution.inspect_wheel(self.wheel(directory, **kwargs))

    def test_source_closure_contains_nested_workspace_manifest_and_sources(self):
        expected = distribution.source_files(ROOT)
        for member in ('crates/qspquest-core', 'crates/qspquest-modeling',
                       'bindings/wasm', 'bindings/cli', 'bindings/mcp',
                       'bindings/python', 'bindings/r/src/rust'):
            self.assertIn(member + '/Cargo.toml', expected)
            self.assertTrue(any(name.startswith(member + '/src/') for name in expected), member)
        self.assertIn('bindings/mcp/examples/parity_audit.rs', expected)
        self.assertIn('bindings/wasm/js/engine-worker.mjs', expected)

    def test_safe_names_and_encoded_size_limits(self):
        for name in ('/absolute', 'a//b', 'a/./b', 'a/../b', 'C:/a', 'a\\b', '\x00'):
            with self.subTest(name=name):
                with self.assertRaises(ValueError):
                    distribution.safe_name(name)
        with self.assertRaisesRegex(ValueError, 'expanded size'):
            distribution.bounded_members([distribution.MAX_MEMBER_BYTES + 1], lambda value: value)


if __name__ == '__main__':
    unittest.main()
