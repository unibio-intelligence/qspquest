"""Refuse documentation drift and links missing from their delivery context."""
import importlib
import importlib.util
import json
from pathlib import Path
import shutil
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
                     'exposure', 'derivatives', 'report', 'interchange', 'individual'):
            paths[f'bindings/python/python/qspquest/{name}.py'] = '"""API fixture."""\n'
        for name, text in paths.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root

    @unittest.skipUnless(importlib.util.find_spec('mistune'), 'install the docs extra to test site rendering')
    def test_site_builders_render_linked_security_policy(self):
        for name in ('build_docs', 'build_quickstart_docs'):
            with self.subTest(builder=name), tempfile.TemporaryDirectory() as directory:
                root = self.site_source(Path(directory) / 'source')
                output = Path(directory) / 'site'
                builder = importlib.import_module(name)
                with patch.object(builder, 'ROOT', root):
                    builder.build(output)
                self.assertIn('href="reference-SECURITY.html#reporting"',
                              (output / 'reference-README.html').read_text())
                self.assertIn('id="reporting"',
                              (output / 'reference-SECURITY.html').read_text())
                check_html(output)

    @unittest.skipUnless(importlib.util.find_spec('mistune'), 'install the docs extra to test site rendering')
    def test_site_builders_still_refuse_private_root_links(self):
        for name in ('build_docs', 'build_quickstart_docs'):
            with self.subTest(builder=name), tempfile.TemporaryDirectory() as directory:
                root = self.site_source(Path(directory) / 'source')
                (root / 'README.internal.md').write_text('# Private notes\n')
                (root / 'README.md').write_text('# Overview\n[Private](README.internal.md)\n')
                builder = importlib.import_module(name)
                with patch.object(builder, 'ROOT', root):
                    with self.assertRaisesRegex(ValueError, 'outside the public documentation roots'):
                        builder.build(Path(directory) / 'site')

    def source(self, directory):
        root = Path(directory)
        index = json.loads((ROOT / 'bindings/mcp/assets/documentation.json').read_text())
        paths = {'Cargo.toml', 'bindings/mcp/assets/documentation.json',
                 'bindings/python/python/qspquest/SECURITY.md'}
        for entry in index['resources']:
            paths.update((entry['source'], 'bindings/mcp/assets/' + entry['asset']))
        for name in paths:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, path)
        return root

    def test_full_candidate_and_embedded_links_match(self):
        report = check(ROOT)
        self.assertEqual(report['status'], 'matched')
        index = json.loads((ROOT / 'bindings/mcp/assets/documentation.json').read_text())
        self.assertEqual(report['generated_files'], len(index['resources']) + 2)
        self.assertGreaterEqual(report['local_links'], 179)
        self.assertGreaterEqual(report['embedded_resource_links'], 29)

    def test_embedded_guide_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            path = root / 'bindings/mcp/assets/guides/spec/language.md'
            path.write_text(path.read_text() + '\nAltered copied guide.\n')
            with self.assertRaisesRegex(ValueError, 'copies differ'):
                check(root)

    def test_embedded_schema_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            path = root / 'bindings/mcp/assets/run.schema.json'
            value = json.loads(path.read_text())
            value['definitions']['Solver']['enum'].append('not_a_solver')
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'copies differ'):
                check(root)

    def test_installed_security_policy_drift_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.source(directory)
            (root / 'bindings/python/python/qspquest/SECURITY.md').write_text('# Wrong policy\n')
            with self.assertRaisesRegex(ValueError, 'copies differ'):
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


if __name__ == '__main__':
    unittest.main()
