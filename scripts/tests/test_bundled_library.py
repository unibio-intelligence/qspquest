"""Keep the bundled model identities, archived fitting example and evidence coherent."""
import hashlib
import json
from pathlib import Path
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
LIBRARY = ROOT / 'bindings/mcp/assets/library'


class BundledLibraryTests(unittest.TestCase):
    def test_selected_versions_and_complete_archive_evidence(self):
        catalog = json.loads((LIBRARY / 'catalog.json').read_text())
        availability_path = LIBRARY / 'qspquest_models/availability.json'
        availability = json.loads(availability_path.read_text())['records']
        self.assertEqual(catalog['availability_sha256'], hashlib.sha256(availability_path.read_bytes()).hexdigest())
        selected = {row['id']: row['version'] for row in catalog['models']}
        self.assertEqual(selected['qspquest_diffdose_2026_idr_benchmark'], '0.3.0')
        self.assertEqual(selected['qspquest_liu_2026_aav_mab_targetx_rat_brain_full_qsp'], '2.1.2')
        self.assertEqual(sum(len(row['scenario_ids']) for row in catalog['models']), 47)
        for row in catalog['models']:
            with self.subTest(model=row['id']):
                for name in ('precision_profiles', 'scientific_assessments', 'protocol_initialization'):
                    if name in row:
                        descriptor = row[name]
                        self.assertEqual(hashlib.sha256((LIBRARY / descriptor['path']).read_bytes()).hexdigest(), descriptor['sha256'])
                key = row['id'] + '/' + row['version']
                archive = LIBRARY / row['archive']
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                self.assertEqual(digest, row['omex_sha256'])
                self.assertEqual(digest, availability[key]['archive_sha256'])
                self.assertEqual(availability[key]['selected_version'], row['version'])
                evidence = archive.with_name('evidence.json').read_bytes()
                self.assertEqual('sha256:' + hashlib.sha256(evidence).hexdigest(), availability[key]['evidence_id'])
                with zipfile.ZipFile(archive) as package:
                    index = json.loads(package.read('index.json'))
                    self.assertEqual(index['model_id'], row['id'])
                    self.assertEqual(index['model_version'], row['version'])
                    self.assertEqual(set(index['native_execution']['scenarios']), set(row['scenario_ids']))
                    for name, sha in index['members'].items():
                        self.assertEqual(hashlib.sha256(package.read(name)).hexdigest(), sha)

    def test_bundled_fitting_request_is_one_complete_archived_workflow(self):
        example = json.loads((ROOT / 'bindings/mcp/assets/fits/library-scalar.json').read_text())
        selected = example['arguments']['model']['library']
        archive = LIBRARY / 'models' / selected['model_id'] / selected['version'] / 'model.omex'
        with zipfile.ZipFile(archive) as package:
            fitting = json.loads(package.read('fitting.json'))
            self.assertEqual(archive.with_name('LICENSE.source').read_bytes(), package.read('LICENSE.source'))
        self.assertEqual(fitting['schema'], 'qspquest.library-fitting/v1')
        self.assertEqual(fitting['model_version'], selected['version'])
        self.assertTrue(fitting['fit_ready'])
        workflow = next(item for item in fitting['workflows'] if item['id'] == example['workflow_id'])
        self.assertEqual(workflow['qualification']['status'], 'passed')
        self.assertEqual(example['tool'], workflow['operation'])
        self.assertEqual(example['arguments']['fit_request'], workflow['native_request'])
        self.assertEqual(example['expected_parameters'], {p['native_name']: p['synthetic_truth'] for p in workflow['parameters']})
        self.assertLessEqual(example['absolute_tolerance'], min(p['recovery_absolute_tolerance'] for p in workflow['parameters']))


if __name__ == '__main__':
    unittest.main()
