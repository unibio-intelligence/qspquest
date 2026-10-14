"""Validate exported tables before numerical analysis and preserve evidence scope."""
import csv
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'docs/examples'))
from table_bundle import read_table, RULE, SCHEMA


class TableBundleTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'run.csv'
        self.names = ['time_h', 'side', 'free,"drug"']
        self.manifest = {
            'schema': SCHEMA, 'producer': 'mcp',
            'table': {'filename': 'run.csv', 'sha256': '', 'bytes': 0, 'rows': 0,
                      'columns': [dict(name=name, unit=unit, kind=kind, native_name=native, meaning=None)
                                  for name, unit, kind, native in zip(self.names, ['h', None, '1'],
                                      ['time', 'event_side', 'output'], [None, None, 'cp'])]},
            'events': {'rule': RULE, 'pre': 1, 'post': 2, 'event_pairs': 1},
            'model': {'id': 'synthetic', 'version': '1', 'archive_sha256': 'a' * 64,
                      'provenance': {'source': 'edited model', 'metadata': {}}},
            'engine_identity': {'compiler_source_sha256': 'fixture'},
            'inputs': {'effective_request': {'regimen': {'end': {'value': 1, 'unit': 'h'}}},
                       'selected_outputs': ['cp'], 'effective_request_hash': 'opaque',
                       'scenario_id': 'preset', 'modified': True},
            'qualification': {'qualified': False, 'status': 'not_assessed',
                              'scope': 'current_engine_and_effective_request',
                              'unit_status': 'source_scale_unqualified',
                              'coordinates': 'unchanged_source_numeric_values',
                              'recorded_dimensional_qualification': False,
                              'recorded_availability': {'current_engine_qualification': 'not_assessed'}}}
        self.write([['0', 'post', '0'], ['1', 'pre', '1'], ['1', 'post', '2']])

    def save_manifest(self):
        Path(str(self.path) + '.manifest.json').write_text(json.dumps(self.manifest))

    def write(self, rows, names=None):
        stream = io.StringIO(newline='')
        writer = csv.writer(stream)
        writer.writerow(self.names if names is None else names)
        writer.writerows(rows)
        data = stream.getvalue().encode()
        self.path.write_bytes(data)
        self.manifest['table'].update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data), rows=len(rows))
        self.save_manifest()

    def test_reimport_preserves_quoted_units_inputs_and_unqualified_source_context(self):
        value = read_table(self.path)
        self.assertEqual(value['manifest'], self.manifest)
        self.assertEqual(list(value['rows'][0]), self.names)
        self.assertEqual(value['rows'][2]['free,"drug"'], 2.0)
        self.assertTrue(value['manifest']['inputs']['modified'])
        self.assertFalse(value['manifest']['qualification']['qualified'])

    def test_edited_bytes_and_missing_sidecar_are_refused(self):
        self.path.write_bytes(self.path.read_bytes() + b'\n')
        with self.assertRaisesRegex(ValueError, 'SHA-256'):
            read_table(self.path)
        Path(str(self.path) + '.manifest.json').unlink()
        with self.assertRaises((OSError, ValueError)):
            read_table(self.path)

    def test_false_metadata_claims_are_refused(self):
        cases = [('rows', 4), ('bytes', True), ('bytes', 256 * 1024 * 1024 + 1), ('filename', 'renamed.csv')]
        for key, value in cases:
            with self.subTest(key=key, value=value):
                before = self.manifest['table'][key]
                self.manifest['table'][key] = value
                self.save_manifest()
                with self.assertRaises(ValueError):
                    read_table(self.path)
                self.manifest['table'][key] = before
        self.manifest['events']['event_pairs'] = 0
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, 'event counts'):
            read_table(self.path)

    def test_rehashed_invalid_rows_are_refused(self):
        for rows in [[['1', 'post', '1'], ['0', 'post', '2']],
                     [['1', 'post', '1'], ['1', 'pre', '2']],
                     [['1', 'pre', '1'], ['1', 'post', '2'], ['1', 'post', '3']],
                     [['NaN', 'post', '1']], [['0', 'post', 'Infinity']],
                     [['0', 'during', '1']], [['0', 'post']], [['0', 'post', '1', '2']]]:
            with self.subTest(rows=rows):
                self.write(rows)
                with self.assertRaises(ValueError):
                    read_table(self.path)

    def test_header_and_duplicate_names_are_refused(self):
        self.write([['0', 'post', '0']], names=['time_h', 'side', 'wrong'])
        with self.assertRaisesRegex(ValueError, 'header'):
            read_table(self.path)
        self.manifest['table']['columns'][2]['name'] = 'side'
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, 'unique'):
            read_table(self.path)

    def test_closed_records_and_qualification_promotion_are_refused(self):
        for record, key, value in [(self.manifest, 'unknown', 1),
                                   (self.manifest['qualification'], 'qualified', True),
                                   (self.manifest['qualification'], 'unit_status', 'qualified'),
                                   (self.manifest['inputs'], 'modified', 'false')]:
            with self.subTest(key=key):
                before = record.get(key)
                record[key] = value
                self.save_manifest()
                with self.assertRaises(ValueError):
                    read_table(self.path)
                if before is None:
                    record.pop(key)
                else:
                    record[key] = before

    def test_duplicate_json_and_nonfinite_provenance_are_refused(self):
        manifest = Path(str(self.path) + '.manifest.json')
        manifest.write_text('{"schema":"ignored",' + json.dumps(self.manifest)[1:])
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            read_table(self.path)
        self.manifest['model']['provenance']['value'] = float('nan')
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, 'Nonfinite'):
            read_table(self.path)

    def test_manual_null_context_and_empty_table_are_explicit(self):
        self.manifest.update(producer='model_library', engine_identity=None)
        self.manifest['model'] = dict(id=None, version=None, archive_sha256=None, provenance=None)
        self.manifest['inputs'] = dict.fromkeys(self.manifest['inputs'])
        self.manifest['events'].update(pre=0, post=0, event_pairs=0)
        self.write([])
        result = read_table(self.path)
        self.assertEqual(result['rows'], [])
        self.assertIsNone(result['manifest']['inputs']['effective_request'])


if __name__ == '__main__':
    unittest.main()
