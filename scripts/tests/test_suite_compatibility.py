"""Suite integrity regressions using synthetic, in-memory observations."""
import importlib.util
import gzip
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location('compatibility', Path(__file__).resolve().parents[1] / 'check_suite_compatibility.py')
SUITE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SUITE)


class SuiteCompatibilityTests(unittest.TestCase):
    def setUp(self):
        self.native = {'outputs': {'cp': {'native_name': 'source_output_0', 'unit': '1'}}}
        self.output = {'schema': 'qspquest.result/v0.1', 'times': [0, 1], 'sides': ['post', 'pre'],
                       'time_unit': 'h',
                       'outputs': [{'name': 'source_output_0', 'unit': '1', 'values': [2, 1]}]}
        self.csv = b'time,side,cp\n0,post,2\n1,pre,1\n'
        self.model_outputs = [{'name': 'source_output_0', 'unit': '1'}]

    def compare(self):
        return SUITE.compare(self.output, self.native, self.csv, 1e-8, 1e-4,
                             time_unit='h', model_outputs=self.model_outputs)

    def git(self, root, *args):
        environment = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
        return subprocess.run(['git', '-c', 'user.name=Test fixture', '-c', 'user.email=test@example.invalid',
                               '-c', 'commit.gpgsign=false', '-C', str(root), *args],
                              env=environment, check=True, capture_output=True).stdout.decode().strip()

    def checkout(self, root):
        root.mkdir()
        (root / 'scripts').mkdir()
        (root / 'scripts/verify.py').write_text('print("verified")\n')
        (root / 'loader.py').write_text('# synthetic loader\n')
        (root / '.gitignore').write_text('__pycache__/\n')
        self.git(root, 'init', '-q')
        self.git(root, 'add', '.')
        self.git(root, 'commit', '-qm', 'Synthetic suite input')
        return self.git(root, 'rev-parse', 'HEAD')

    def test_validation_requires_exact_compiler_source(self):
        environment = {"engine": {"compiler_source_sha256": "sha256:" + "1" * 64}}
        identity = {"compiler_version": "qspquest/0.1.0", "compiler_source_sha256": "sha256:" + "1" * 64}
        SUITE.check_validation_engine(identity, environment)
        for source in ["sha256:" + "2" * 64, None]:
            with self.subTest(source=source), self.assertRaisesRegex(ValueError, "compiler source"):
                SUITE.check_validation_engine(dict(identity, compiler_source_sha256=source), environment)

    def test_checkout_identity_and_ignored_cache(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'models'
            commit = self.checkout(root)
            expected = {'commit': commit, 'tree': self.git(root, 'rev-parse', 'HEAD^{tree}'), 'tracked_files': 3}
            self.assertEqual(SUITE.verify_checkout(root, commit), expected)
            (root / '__pycache__').mkdir()
            (root / '__pycache__/loader.pyc').write_bytes(b'ignored cache')
            self.assertEqual(SUITE.verify_checkout(root, commit), expected)
            (root / 'unexpected.py').write_text('print("untracked")\n')
            with self.assertRaisesRegex(ValueError, 'must be clean'):
                SUITE.verify_checkout(root, commit)

    def test_checkout_rejects_wrong_commit_nonrepo_and_subdirectory(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'models'
            commit = self.checkout(root)
            for path, pin, message in [(root, '0' * 40, 'commit differs'),
                                       (Path(temp), commit, 'must be a Git checkout'),
                                       (root / 'scripts', commit, 'checkout root')]:
                with self.subTest(path=path, pin=pin), self.assertRaisesRegex(ValueError, message):
                    SUITE.verify_checkout(path, pin)

    def test_checkout_rejects_changed_staged_deleted_and_hidden_tracked_files(self):
        for mutation in ('verifier', 'loader', 'staged', 'deleted', 'assume-unchanged', 'skip-worktree', 'index-only'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temp:
                root = Path(temp) / 'models'
                commit = self.checkout(root)
                target = root / ('scripts/verify.py' if mutation == 'verifier' else 'loader.py')
                if mutation in ('assume-unchanged', 'skip-worktree'):
                    self.git(root, 'update-index', '--' + mutation, 'loader.py')
                original = target.read_text()
                if mutation == 'deleted':
                    target.unlink()
                else:
                    target.write_text('# changed source\n')
                if mutation in ('staged', 'index-only'):
                    self.git(root, 'add', 'loader.py')
                if mutation == 'index-only':
                    target.write_text(original)
                with self.assertRaisesRegex(ValueError, 'must be clean|differs from its commit'):
                    SUITE.verify_checkout(root, commit)

    def test_comparator_rejects_incomplete_duplicate_and_empty_reference_rows(self):
        for raw in (b'', b'time,side,cp\n', b'time,side,cp,cp\n0,post,999,2\n1,pre,999,1\n',
                    b'time,side,cp\n0,post,2,extra\n1,pre,1\n',
                    b'time,side,cp\n0,post\n1,pre,1\n', b'time,side\n0,post\n1,pre\n',
                    b'time,side,cp\n0,post,2\n\n1,pre,1\n'):
            with self.subTest(reference=raw), self.assertRaises(ValueError):
                self.csv = raw
                self.compare()

    def test_comparator_rejects_boolean_nonfinite_and_malformed_output_values(self):
        for value in (True, False, None, '1', float('nan'), float('inf'), 10**1000):
            with self.subTest(value=repr(value)[:40]), self.assertRaises(ValueError):
                self.output['outputs'][0]['values'][1] = value
                self.compare()
        self.output['outputs'][0]['values'] = [2]
        with self.assertRaisesRegex(ValueError, 'observation count'):
            self.compare()

    def test_comparator_checks_time_units_grid_and_event_side_shape(self):
        for times in ([False, 1], [0, True], [0, float('inf')], [0], None):
            with self.subTest(times=times), self.assertRaises(ValueError):
                self.output['times'] = times
                self.compare()
        self.output['times'] = [0, 1]
        for sides in (['post', 'unknown'], ['post'], None):
            with self.subTest(sides=sides), self.assertRaises(ValueError):
                self.output['sides'] = sides
                self.compare()
        self.output['sides'] = ['post', 'pre']
        self.output['time_unit'] = 'd'
        with self.assertRaisesRegex(ValueError, 'time unit'):
            self.compare()

    def test_comparator_rejects_invalid_reference_grid_and_arithmetic(self):
        for times, sides in (([1, 0], ['post', 'pre']), ([0, 0], ['post', 'pre']),
                             ([0, float('inf')], ['post', 'pre']), ([0, 1], ['bad', 'pre'])):
            with self.subTest(times=times, sides=sides), self.assertRaises(ValueError):
                self.output['times'], self.output['sides'] = times, sides
                self.csv = f'time,side,cp\n{times[0]},{sides[0]},2\n{times[1]},{sides[1]},1\n'.encode()
                self.compare()
        self.output['times'], self.output['sides'] = [0, 1], ['post', 'pre']
        self.csv = b'time,side,cp\n0,post,2\n1,pre,1e308\n'
        self.output['outputs'][0]['values'][1] = -1e308
        with self.assertRaisesRegex(ValueError, 'arithmetic overflow'):
            self.compare()

    def test_comparator_accepts_explicit_pre_post_jump_and_rejects_extra_outputs(self):
        self.output['times'], self.output['sides'] = [0, 0], ['pre', 'post']
        self.csv = b'time,side,cp\n0,pre,2\n0,post,1\n'
        self.assertEqual(self.compare()['observations'], 2)
        self.output['outputs'].append({'name': 'extra', 'unit': '1', 'values': [2, 1]})
        with self.assertRaisesRegex(ValueError, 'Unexpected native output'):
            self.compare()

    def test_comparator_accepts_and_checks_declared_state_outputs_without_reference_columns(self):
        self.model_outputs.append({'name': 'state', 'unit': 'mg'})
        self.output['outputs'].append({'name': 'state', 'unit': 'mg', 'values': [6, 3]})
        self.assertEqual(self.compare()['output_values'], 2)
        self.output['outputs'][1]['values'] = [6]
        with self.assertRaisesRegex(ValueError, 'observation count'):
            self.compare()

    def test_comparator_rejects_nonfinite_references_alias_collisions_and_malformed_columns(self):
        for value in ('nan', 'inf', '-inf', 'not-a-number'):
            with self.subTest(reference=value), self.assertRaises(ValueError):
                self.csv = f'time,side,cp\n0,post,2\n1,pre,{value}\n'.encode()
                self.compare()
        self.csv = b'time,side,cp\n0,post,2\n1,pre,1\n'
        columns = self.output['outputs']
        for bad in (None, {}, [None], [{'name': None}], [{'name': 'source_output_0'}]):
            with self.subTest(columns=bad), self.assertRaises(ValueError):
                self.output['outputs'] = bad
                self.compare()
        self.output['outputs'] = columns
        self.native['outputs']['other'] = dict(self.native['outputs']['cp'])
        self.csv = b'time,side,cp,other\n0,post,2,2\n1,pre,1,1\n'
        with self.assertRaisesRegex(ValueError, 'aliases must be distinct'):
            self.compare()

    def suite_fixture(self, root, *, mutating_verifier=False):
        models, validation = root / 'models', root / 'validation'
        self.checkout(models)
        self.checkout(validation)
        request = {'solver': 'diffsol_bdf'}
        native = {'schema': 'qspquest.models.native-execution/v1', 'model': 'native/model.json',
                  'scenarios': {'example': 'native/run.json'}, **self.native}
        with zipfile.ZipFile(models / 'model.omex', 'w') as archive:
            archive.writestr('index.json', json.dumps({'model_id': 'synthetic', 'model_version': '1', 'native_execution': native}))
            archive.writestr('native/model.json', json.dumps({'time_unit': 'h', 'outputs': self.model_outputs}))
            archive.writestr('native/run.json', json.dumps(request))
        digest = SUITE.sha((models / 'model.omex').read_bytes())
        reference = gzip.compress(self.csv)
        (models / 'reference.csv.gz').write_bytes(reference)
        evidence = {'omex_sha256': digest, 'cases': [{'scenario_id': 'example', 'engine_identity': {},
                    'files': {'qspquest_csv': {'compression': 'gzip', 'path': 'reference.csv.gz',
                    'sha256': SUITE.sha(reference), 'uncompressed_sha256': SUITE.sha(self.csv)}}}]}
        (models / 'evidence.json').write_text(json.dumps(evidence))
        (models / 'catalog.json').write_text(json.dumps({'models': [{'id': 'synthetic', 'version': '1',
                                                                   'archive': 'model.omex', 'omex_sha256': digest}]}))
        (models / 'pyproject.toml').write_text('[project]\nversion="0.1.0"\n')
        library = root / 'bindings/mcp/assets/library'
        library.mkdir(parents=True)
        (library / 'model.omex').write_bytes((models / 'model.omex').read_bytes())
        (library / 'catalog.json').write_bytes((models / 'catalog.json').read_bytes())
        self.pin_bundle_catalog(library)
        verifier = 'print("verified")\n'
        if mutating_verifier:
            verifier = 'from pathlib import Path\nPath("loader.py").write_text("# changed during verification\\n")\n' + verifier
        (models / 'scripts/verify_library.py').write_text(verifier)
        (validation / 'environment.json').write_text(json.dumps({'engine': {'compiler_source_sha256': 'sha256:' + '1' * 64}}))
        (validation / 'manifest.json').write_text(json.dumps({'files': {'environment.json': SUITE.sha((validation / 'environment.json').read_bytes())}}))
        for checkout in (models, validation):
            self.git(checkout, 'add', '.')
            self.git(checkout, 'commit', '-qm', 'Add synthetic verification inputs')
        pins = {'models': {'commit': self.git(models, 'rev-parse', 'HEAD'), 'version': '0.1.0',
                           'catalog_sha256': SUITE.sha((models / 'catalog.json').read_bytes())},
                'validation': {'commit': self.git(validation, 'rev-parse', 'HEAD'),
                               'manifest_sha256': SUITE.sha((validation / 'manifest.json').read_bytes())}}
        lock = {'components': pins, 'required_engine_files': [], 'engine_version': '0.1.0',
                'case': {'model_id': 'synthetic', 'model_version': '1', 'scenario': 'example',
                         'solvers': ['diffsol_bdf', 'diffsol_rodas5p'], 'absolute_tolerance': 1e-8, 'relative_tolerance': 1e-4}}
        cli = root / 'cli'
        output = {**self.output, 'identity': {'compiler_version': 'qspquest/0.1.0', 'compiler_source_sha256': 'sha256:' + '1' * 64}}
        cli.write_text('#!' + sys.executable + '\nprint(' + repr(json.dumps(output)) + ')\n')
        cli.chmod(0o755)
        return lock, models, validation, cli

    def pin_bundle_catalog(self, library):
        (library / 'version.json').write_text(json.dumps({
            'catalog_sha256': SUITE.sha((library / 'catalog.json').read_bytes())}))

    def test_every_bundled_archive_matches_even_when_each_catalog_is_self_consistent(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lock, models, validation, cli = self.suite_fixture(root)
            catalog = json.loads((models / 'catalog.json').read_text())
            library = root / 'bindings/mcp/assets/library'
            self.assertEqual(len(SUITE.check_bundled_models(models, catalog, root)), 1)
            # A second model outside the selected replay has identical native input
            # but different scenario XML, and both catalogs pin their own archive.
            for folder, xml in ((models, '<model value="1"/>'), (library, '<model value="2"/>')):
                with zipfile.ZipFile(folder / 'second.omex', 'w') as archive:
                    archive.writestr('native/model.json', '{"same":"native model"}')
                    archive.writestr('scenarios/example/model.xml', xml)
                value = json.loads((folder / 'catalog.json').read_text())
                value['models'].append({'id': 'second', 'version': '1', 'archive': 'second.omex',
                                       'omex_sha256': SUITE.sha((folder / 'second.omex').read_bytes())})
                (folder / 'catalog.json').write_text(json.dumps(value))
            self.pin_bundle_catalog(library)
            catalog = json.loads((models / 'catalog.json').read_text())
            with self.assertRaisesRegex(ValueError, 'archive differs.*second@1'):
                SUITE.check_bundled_models(models, catalog, root)

    def test_bundle_rejects_missing_versions_duplicates_empty_catalogs_and_stale_pins(self):
        for mutation in ('missing-version', 'duplicate-bundle', 'duplicate-published', 'empty',
                         'bundle-bytes', 'published-bytes', 'catalog-pin'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                self.suite_fixture(root)
                library = root / 'bindings/mcp/assets/library'
                models = root / 'models'
                published = json.loads((models / 'catalog.json').read_text())
                bundled = json.loads((library / 'catalog.json').read_text())
                if mutation == 'missing-version':
                    bundled['models'][0]['version'] = 'unpublished'
                elif mutation == 'duplicate-bundle':
                    bundled['models'] *= 2
                elif mutation == 'duplicate-published':
                    published['models'] *= 2
                elif mutation == 'empty':
                    bundled['models'] = []
                elif mutation in ('bundle-bytes', 'published-bytes'):
                    folder = library if mutation == 'bundle-bytes' else models
                    (folder / 'model.omex').write_bytes(b'changed archive')
                (library / 'catalog.json').write_text(json.dumps(bundled, indent=2))
                if mutation != 'catalog-pin':
                    self.pin_bundle_catalog(library)
                with self.assertRaises(ValueError):
                    SUITE.check_bundled_models(models, published, root)

    @unittest.skipIf(os.name == 'nt', 'synthetic executable uses a POSIX shebang')
    def test_complete_suite_records_verified_commits_and_rechecks_sources(self):
        for mutating in (False, True):
            with self.subTest(mutating=mutating), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                lock, models, validation, cli = self.suite_fixture(root, mutating_verifier=mutating)
                report = {'checks': []}
                if mutating:
                    with self.assertRaisesRegex(ValueError, 'must be clean'):
                        SUITE.check(lock, models, validation, cli, report, engine_root=root)
                    self.assertNotIn('counterpart_recheck_passed', report)
                else:
                    SUITE.check(lock, models, validation, cli, report, engine_root=root)
                    self.assertTrue(report['counterpart_recheck_passed'])
                    self.assertEqual(len(report['checks']), 4)
                    for role in ('models', 'validation'):
                        self.assertEqual(report['verified_counterparts'][role]['commit'], lock['components'][role]['commit'])

    def test_wrong_commit_stops_before_counterpart_verifiers(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lock, models, validation, cli = self.suite_fixture(root)
            lock['components']['models']['commit'] = '0' * 40
            with patch.object(SUITE, 'stored_check') as verifier:
                with self.assertRaisesRegex(ValueError, 'commit differs'):
                    SUITE.check(lock, models, validation, cli, {'checks': []}, engine_root=root)
                verifier.assert_not_called()

    def test_native_alias_mapping_preserves_exact_grid_and_sides(self):
        self.assertEqual(self.compare(), {'observations': 2, 'output_values': 2, 'maximum_absolute_error': 0})

    def test_shifted_grid_or_event_side_is_rejected(self):
        self.output['times'][1] = 1.01
        with self.assertRaisesRegex(ValueError, 'times or event sides'):
            self.compare()
        self.output['times'][1] = 1
        self.output['sides'][1] = 'post'
        with self.assertRaisesRegex(ValueError, 'times or event sides'):
            self.compare()

    def test_out_of_tolerance_and_nonfinite_predictions_are_rejected(self):
        for value in [1.001, float('nan'), float('inf')]:
            self.output['outputs'][0]['values'][1] = value
            with self.assertRaises(ValueError):
                self.compare()

    def test_missing_outputs_wrong_units_and_duplicate_columns_are_rejected(self):
        self.output['outputs'] = []
        with self.assertRaisesRegex(ValueError, 'Missing native output'):
            self.compare()
        self.output['outputs'] = [{'name': 'source_output_0', 'unit': '1', 'values': [2, 1]}]
        self.output['outputs'][0]['unit'] = 'mg/L'
        with self.assertRaisesRegex(ValueError, 'Output units'):
            self.compare()
        self.output['outputs'][0]['unit'] = '1'
        self.output['outputs'].append(self.output['outputs'][0])
        with self.assertRaisesRegex(ValueError, 'Duplicate native output'):
            self.compare()

    def test_corrupt_file_and_symlink_are_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            raw = b'pinned synthetic input'
            (root / 'input.json').write_bytes(raw)
            self.assertEqual(SUITE.checked(root, 'input.json', SUITE.sha(raw)), raw)
            (root / 'input.json').write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError, 'Pinned checksum differs'):
                SUITE.checked(root, 'input.json', SUITE.sha(raw))
            (root / 'alias.json').symlink_to('input.json')
            with self.assertRaisesRegex(ValueError, 'regular suite file'):
                SUITE.checked(root, 'alias.json', SUITE.sha(raw))

    def test_missing_tutorial_input_fails_before_verifiers_or_cli(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lock = {'required_engine_files': ['docs/missing-example.json']}
            with self.assertRaisesRegex(ValueError, 'tutorial input is missing'):
                SUITE.check(lock, root, root, root / 'absent-cli', {'checks': []}, engine_root=root)

    def test_mismatched_library_version_fails_before_execution(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            catalog = b'{"models":[]}'
            manifest = b'{"files":{}}'
            (root / 'catalog.json').write_bytes(catalog)
            (root / 'manifest.json').write_bytes(manifest)
            (root / 'pyproject.toml').write_text('[project]\nversion="9.0.0"\n')
            lock = {'required_engine_files': [], 'components': {
                'models': {'catalog_sha256': SUITE.sha(catalog), 'version': '0.1.0'},
                'validation': {'manifest_sha256': SUITE.sha(manifest)}}}
            for pin in lock['components'].values():
                pin['commit'] = '0' * 40
            with patch.object(SUITE, 'verify_checkout', return_value={}):
                with self.assertRaisesRegex(ValueError, 'package version differs'):
                    SUITE.check(lock, root, root, root / 'absent-cli', {'checks': []}, engine_root=root)

    def test_traversal_and_mutable_counterpart_refs_are_refused(self):
        for name in ['../private.json', '/private.json', 'dir/../private.json', 'dir\\private.json']:
            with self.assertRaisesRegex(ValueError, 'Unsafe suite file reference'):
                SUITE.relative(name)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'lock.json'
            path.write_text(json.dumps({'schema': 'qspquest.suite-compatibility/v1', 'components': {
                'models': {'repository': 'unibio-intelligence/qspquest-models', 'commit': 'main'}}}))
            with self.assertRaisesRegex(ValueError, 'full immutable ID'):
                SUITE.load_lock(path)

    def test_invalid_lock_shape_and_boolean_tolerance_are_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'lock.json'
            path.write_text('[]')
            with self.assertRaisesRegex(ValueError, 'Unsupported suite'):
                SUITE.load_lock(path)
            lock = json.loads((Path(__file__).resolve().parents[2] / '.github/suite-compatibility.json').read_text())
            lock['case']['absolute_tolerance'] = True
            path.write_text(json.dumps(lock))
            with self.assertRaisesRegex(ValueError, 'Invalid comparison tolerance'):
                SUITE.load_lock(path)


if __name__ == '__main__':
    unittest.main()
