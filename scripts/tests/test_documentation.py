"""Refuse documentation drift and links missing from their delivery context."""
import re
import importlib
import importlib.util
import json
from pathlib import Path
import shutil
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from check_documentation import check
from doc_links import anchors, check_html
from generate_mcp_resources import generated


class DocumentationTests(unittest.TestCase):
    def site_source(self, directory):
        root = Path(directory).resolve()
        paths = {
            'docs/site/index.md': '# Start\n[Overview](../../README.md)\n',
            'README.md': '# Overview\n[Report a vulnerability](SECURITY.md#reporting)\n',
            'SECURITY.md': '# Security\n## Reporting\nReport privately.\n',
            'LICENSE': 'Synthetic test fixture\n',
            'docs/examples/python_quickstart.py': '',
            'docs/examples/interchange_roundtrip.py': '',
            'examples/synthetic/absorption.qsp': '',
            'examples/synthetic/absorption.model.json': '{}',
        }
        for name in ('installation', 'ANALYSIS', 'PYTHON-USER-GUIDE',
                     'R-USER-GUIDE', 'SAEM', 'modeling', 'fitting'):
            paths[f'docs/{name}.md'] = '# Guide\n'
        paths['examples/synthetic/README.md'] = '# Synthetic model\n'
        for name in ('__init__', 'friendly', 'control', 'doctor', 'dataset',
                     'estimation', 'population', 'diagnostics', 'uncertainty',
                     'exposure', 'nca', 'derivatives', 'report', 'interchange',
                     'individual', 'tables', 'plot/__init__', 'plot/style', 'plot/pkpd'):
            paths[f'bindings/python/python/qspquest/{name}.py'] = '"""API fixture."""\n'
        for name, text in paths.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root

    @unittest.skipUnless(importlib.util.find_spec('mistune'), 'install the docs extra to test site rendering')
    def test_site_builder_renders_linked_security_policy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.site_source(Path(directory) / 'source')
            output = Path(directory) / 'site'
            builder = importlib.import_module('build_docs')
            with patch.object(builder, 'ROOT', root):
                builder.build(output)
            self.assertIn('href="reference-SECURITY.html#reporting"',
                          (output / 'reference-README.html').read_text())
            self.assertIn('id="reporting"',
                          (output / 'reference-SECURITY.html').read_text())
            check_html(output)

    @unittest.skipUnless(importlib.util.find_spec('mistune'), 'install the docs extra to test site rendering')
    def test_site_builder_still_refuses_private_root_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.site_source(Path(directory) / 'source')
            (root / 'README.internal.md').write_text('# Private notes\n')
            (root / 'README.md').write_text('# Overview\n[Private](README.internal.md)\n')
            builder = importlib.import_module('build_docs')
            with patch.object(builder, 'ROOT', root):
                with self.assertRaisesRegex(ValueError, 'outside the public documentation roots'):
                    builder.build(Path(directory) / 'site')

    def source(self, directory):
        root = Path(directory)
        index = json.loads((ROOT / 'bindings/mcp/assets/documentation.json').read_text())
        paths = {'Cargo.toml', '.dockerignore', 'bindings/mcp/assets/documentation.json',
                 'bindings/mcp/assets/documentation-links.json',
                 'bindings/python/python/qspquest/SECURITY.md',
                 'bindings/mcp/assets/fits/population.json'}
        paths.update(entry['source'] for entry in index['resources'])
        # Embedded pages may show figures; the generator checks that each image exists.
        for name in [p for p in paths if p.endswith('.md')]:
            for image in re.findall(r'!\[[^\]\n]+\]\(([^\s()]+)\)', (ROOT / name).read_text()):
                if '://' not in image:
                    paths.add(str((Path(name).parent / image).as_posix()))
        for name in paths:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)
        return root

    def test_full_candidate_and_embedded_links_match(self):
        report = check(ROOT)
        self.assertEqual(report['status'], 'matched')
        index = json.loads((ROOT / 'bindings/mcp/assets/documentation.json').read_text())
        self.assertEqual(report['generated_files'], 4)
        self.assertEqual(len(index['resources']), len({entry['source'] for entry in index['resources']}))
        self.assertGreaterEqual(report['local_links'], 179)
        self.assertGreaterEqual(report['embedded_resource_links'], 29)

    def test_body_edits_need_no_regenerated_index(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            path = root / 'spec/language.md'
            path.write_text(path.read_text() + '\nAn added paragraph.\n')
            path = root / 'spec/schema/run.schema.json'
            value = json.loads(path.read_text())
            value['description'] = 'Edited schema description.'
            path.write_text(json.dumps(value))
            path = root / 'spec/capabilities.json'
            value = json.loads(path.read_text())
            value['scope'] += ' Edited.'
            path.write_text(json.dumps(value))
            self.assertEqual(check(root)['status'], 'matched')

    def test_heading_edit_without_regenerating_the_index_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            path = root / 'spec/language.md'
            path.write_text(path.read_text() + '\n## Added section\n')
            with self.assertRaisesRegex(ValueError, 'index differs'):
                check(root)

    def test_installed_security_policy_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            (root / 'bindings/python/python/qspquest/SECURITY.md').write_text('# Wrong policy\n')
            with self.assertRaisesRegex(ValueError, 'index differs'):
                check(root)

    def test_canonical_heading_typo_cannot_generate_a_resource(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            path = root / 'spec/language.md'
            path.write_text(path.read_text() + '\n[Bad section](regimens.md#not-a-heading)\n')
            with self.assertRaisesRegex(ValueError, 'Unresolved heading'):
                generated(root)

    def test_missing_example_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            (root / 'conformance/requests/synthetic-one-compartment.json').unlink()
            with self.assertRaises((ValueError, OSError)):
                generated(root)

    def test_rendered_html_heading_typo_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'index.html').write_text('<a href="guide.html#wrong">Guide</a>')
            (root / 'guide.html').write_text('<h1 id="right">Guide</h1>')
            with self.assertRaisesRegex(ValueError, 'Unresolved generated HTML heading'):
                check_html(root)

    def test_duplicate_headings_and_fenced_links(self):
        self.assertEqual(anchors('# Guide\n## Units\n## Units\n```md\n# Ignored\n```\n'),
                         {'guide', 'units', 'units-1'})


class TutorialExecutionTests(unittest.TestCase):
    def run_tutorials(self, directory, failure=None):
        root = Path(directory)
        scripts = root / 'scripts'
        scripts.mkdir()
        runner = scripts / 'check_tutorial_outputs.py'
        shutil.copyfile(ROOT / 'scripts/check_tutorial_outputs.py', runner)
        examples = root / 'docs/examples'
        examples.mkdir(parents=True)
        tutorials = root / 'docs/tutorials'
        tutorials.mkdir()
        for name in ('single-dose-pk', 'multiple-dose-pk'):
            stem = name.replace('-', '_')
            (tutorials / (name + '.md')).write_text(
                f'```sh\npython docs/examples/{stem}.py --out output\n```\n'
                'prints:\n\n```text\nvalue 1.0\n```\n')
            (examples / (stem + '.py')).write_text('print("value 1.0")\n')
        figure_source = """import os
from pathlib import Path
assert os.environ['MPLBACKEND'] == 'Agg'
Path(os.environ['MPLCONFIGDIR'], 'cache-probe').write_text('writable')
Path(os.environ['QSPQUEST_FIGURE_DIR']).mkdir(parents=True, exist_ok=True)
Path(os.environ['QSPQUEST_FIGURE_DIR'], 'figure.txt').write_text('synthetic figure')
"""
        for name in ('single_dose_pk', 'multiple_dose_pk', 'oral_pk_analysis',
                     'parallel_predictive_checks', 'population_log_wald',
                     'profiled_proportional_errors', 'paired_endpoint_fitting',
                     'discrete_data_fitting', 'inverse_gaussian_input'):
            (examples / (name + '_figures.py')).write_text(figure_source)
        if failure:
            (examples / failure).write_text('import sys\nprint("synthetic failure", file=sys.stderr)\nsys.exit(7)\n')
        return subprocess.run([sys.executable, str(runner)], capture_output=True, text=True,
                              env={**os.environ, 'MPLBACKEND': 'macosx',
                                   'MPLCONFIGDIR': str(root / 'missing/cache')})

    def test_tutorial_figures_override_desktop_backend_and_unwritable_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_tutorials(directory)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('2 tutorials match their documented output', result.stdout)

    def test_example_and_both_figure_loops_propagate_child_failures(self):
        for filename in ('single_dose_pk.py', 'single_dose_pk_figures.py',
                         'oral_pk_analysis_figures.py'):
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as directory:
                result = self.run_tutorials(directory, filename)
                self.assertEqual(result.returncode, 1)
                self.assertIn('exit 7', result.stderr)
                self.assertIn('synthetic failure', result.stderr)
                self.assertNotIn('tutorials match', result.stdout)


if __name__ == '__main__':
    unittest.main()
