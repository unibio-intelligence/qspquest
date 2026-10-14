"""A complete fitting result cannot be replaced by partial or corrupted chunks."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('example_stdio_rpc', Path(__file__).resolve().parents[2] / 'docs/examples/stdio_rpc.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ResourceCompletenessTests(unittest.TestCase):
    def client(self, resources):
        client = object.__new__(module.StdioRPC)
        client.rpc = lambda method, params: {'contents': [{'uri': params['uri'], 'text': resources[params['uri']]}]}
        return client

    def fixture(self, artifact=False):
        text = json.dumps({'parameters': {'cl': {'value': 0.6, 'unit': 'L/h'}}, 'description': 'µ synthetic'}, ensure_ascii=False)
        sha = hashlib.sha256(text.encode('utf-8')).hexdigest()
        manifest = ({'schema': 'qspquest.result-artifact/v0.1', 'chunk_uri_prefix': 'chunks/', 'chunk_count': 2,
                     'bytes': len(text.encode('utf-8')), 'sha256': sha} if artifact else
                    {'kind': 'utf8_chunk_manifest', 'chunks': ['chunks/0', 'chunks/1'], 'sha256': sha})
        resources = {'manifest': json.dumps(manifest), 'chunks/0': text[:30], 'chunks/1': text[30:]}
        return resources, json.loads(text)

    def test_complete_legacy_and_persistent_resources(self):
        for artifact in (False, True):
            resources, expected = self.fixture(artifact)
            self.assertEqual(self.client(resources).read_json('manifest'), expected)

    def test_refuses_changed_bytes_missing_chunk_and_complete_size_overflow(self):
        for mutation in ('changed', 'missing', 'oversize'):
            resources, _ = self.fixture()
            if mutation == 'changed': resources['chunks/1'] += ' '
            if mutation == 'missing': del resources['chunks/1']
            if mutation == 'oversize':
                text = json.dumps({'long': 'x' * 1000})
                manifest = json.loads(resources['manifest'])
                manifest['sha256'] = hashlib.sha256(text.encode()).hexdigest()
                resources.update(manifest=json.dumps(manifest), **{'chunks/0': text[:505], 'chunks/1': text[505:]})
            with self.assertRaises((ValueError, KeyError)):
                self.client(resources).read_json('manifest', max_bytes=700 if mutation == 'oversize' else 10000)

    def test_refuses_impossible_count_or_wrong_byte_count(self):
        for mutation in ('count', 'bytes'):
            resources, _ = self.fixture(True)
            manifest = json.loads(resources['manifest'])
            if mutation == 'count': manifest['chunk_count'] = True
            else: manifest['bytes'] += 1
            resources['manifest'] = json.dumps(manifest)
            with self.assertRaises(ValueError): self.client(resources).read_json('manifest')

    def test_refuses_non_object_in_place_of_complete_result(self):
        with self.assertRaises(ValueError): self.client({'result': '[1,2]'}).read_json('result')


if __name__ == '__main__':
    unittest.main()
