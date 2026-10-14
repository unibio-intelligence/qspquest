"""Independent byte integrity, ordering and expiry regressions; no platform/science claim."""
import base64
from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest

spec = importlib.util.spec_from_file_location('artifact_download', Path(__file__).resolve().parents[2]/'docs/examples/managed_artifact_download.py')
download = importlib.util.module_from_spec(spec)
spec.loader.exec_module(download)


def fixture(raw):
    digest = hashlib.sha256(raw).hexdigest()
    now = int(time.time())
    uri = download.PREFIX+'synthetic/'+digest
    manifest = {'schema': 'qspquest.managed-result-artifact/v1', 'uri': uri, 'artifact_id': 'synthetic',
                'organization_id': 'tenant-fixture', 'project_id': 'project-fixture', 'sha256': digest,
                'bytes': len(raw), 'media_type': 'application/json', 'result_schema': 'synthetic-transport-only',
                'created_at_unix_seconds': now-1, 'expires_at_unix_seconds': now+599, 'retention_seconds': 600,
                'chunk_bytes': download.CHUNK_BYTES, 'chunk_count': (len(raw)+download.CHUNK_BYTES-1)//download.CHUNK_BYTES,
                'chunk_uri_prefix': uri+'/chunks/', 'instructions': 'Synthetic transport fixture; no scientific execution'}
    resources = {uri: manifest}
    for index in range(manifest['chunk_count']):
        offset = index*download.CHUNK_BYTES
        part = raw[offset:offset+download.CHUNK_BYTES]
        resources[manifest['chunk_uri_prefix']+str(index)] = {'schema': 'qspquest.managed-artifact-chunk/v1',
            'artifact_uri': uri, 'artifact_sha256': digest, 'index': index, 'offset': offset,
            'bytes': len(part), 'sha256': hashlib.sha256(part).hexdigest(), 'encoding': 'base64',
            'data': base64.b64encode(part).decode('ascii')}
    return manifest, resources


class DownloadTests(unittest.TestCase):
    def test_complete_large_unicode_binary_and_empty_content(self):
        for raw in [b'a'+('λ🧪'*700_000).encode(), bytes(range(256))*400, b'']:
            manifest, resources = fixture(raw)
            with tempfile.TemporaryDirectory() as parent:
                target = Path(parent)/'result'
                report = download.download(resources.__getitem__, manifest, target)
                self.assertEqual(report['status'], 'passed')
                self.assertEqual((target/'artifact.bin').read_bytes(), raw)
                self.assertFalse((target/'artifact.partial').exists())
                self.assertEqual(report['sha256'], hashlib.sha256(raw).hexdigest())

    def test_corruption_wrong_scope_range_and_boolean_indices_retain_failure(self):
        raw = b'x'*48_001
        manifest, resources = fixture(raw)
        second = manifest['chunk_uri_prefix']+'1'
        changes = [lambda c: c.update(artifact_sha256='0'*64), lambda c: c.update(artifact_uri='qspquest://managed-artifacts/other/'+'0'*64),
                   lambda c: c.update(offset=0), lambda c: c.update(index=True), lambda c: c.update(data='!!!!'),
                   lambda c: c.update(sha256='0'*64), lambda c: c.update(bytes=2)]
        for change in changes:
            altered = deepcopy(resources)
            change(altered[second])
            with tempfile.TemporaryDirectory() as parent:
                target = Path(parent)/'result'
                with self.assertRaises((ValueError, KeyError)):
                    download.download(altered.__getitem__, manifest, target)
                self.assertFalse((target/'artifact.bin').exists())
                self.assertEqual((target/'artifact.partial').read_bytes(), raw[:48_000])
                self.assertEqual(json.loads((target/'checks.json').read_bytes())['status'], 'failed')

    def test_full_digest_truncation_expiration_and_overwrite_are_refused(self):
        manifest, resources = fixture(b'x'*48_001)
        changed = deepcopy(manifest)
        changed['chunk_count'] = 1
        with self.assertRaises(ValueError): download.descriptor(changed)
        changed = deepcopy(manifest)
        changed['created_at_unix_seconds'] -= 1000
        changed['expires_at_unix_seconds'] -= 1000
        with self.assertRaisesRegex(ValueError, 'expired'): download.descriptor(changed)
        changed = deepcopy(manifest)
        changed['bytes'] = True
        with self.assertRaises(ValueError): download.descriptor(changed)
        resources[manifest['chunk_uri_prefix']+'1']['data'] = base64.b64encode(b'y').decode()
        resources[manifest['chunk_uri_prefix']+'1']['sha256'] = hashlib.sha256(b'y').hexdigest()
        with tempfile.TemporaryDirectory() as parent:
            target = Path(parent)/'result'
            with self.assertRaisesRegex(ValueError, 'full SHA-256'):
                download.download(resources.__getitem__, manifest, target)
            with self.assertRaises(FileExistsError):
                download.download(resources.__getitem__, manifest, target)


if __name__ == '__main__':
    unittest.main()
