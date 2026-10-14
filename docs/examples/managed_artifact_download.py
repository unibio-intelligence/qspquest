"""Retrieve a complete retained artifact from QSPQuest's managed JSON HTTP endpoint."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

MAX_RESPONSE = 256_000
MAX_BYTES = 128_000_000
CHUNK_BYTES = 48_000
PREFIX = 'qspquest://managed-artifacts/'


def _json(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('Duplicate artifact JSON field')
            result[key] = value
        return result
    def reject(value):
        raise ValueError('Nonfinite artifact JSON')
    return json.loads(raw, object_pairs_hook=unique, parse_constant=reject)


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class Client:
    """Bounded client for the server's JSON response mode; token is never logged."""
    def __init__(self, endpoint, bearer):
        url = urlsplit(endpoint)
        loopback = url.hostname in ('localhost', '127.0.0.1', '::1')
        if not url.hostname or url.username is not None or url.password is not None or url.query or url.fragment or not (url.scheme == 'https' or url.scheme == 'http' and loopback):
            raise ValueError('Use an HTTPS MCP URL, or HTTP on loopback for the synthetic tutorial; no URL credentials, query or fragment')
        if not bearer or any(c in bearer for c in '\r\n'):
            raise ValueError('Set QSPQUEST_MCP_BEARER to a verified managed session token')
        self.endpoint, self.bearer = endpoint, bearer
        self.opener = build_opener(ProxyHandler({}), NoRedirect())
        self.identifier = 0

    def rpc(self, method, params, notify=False):
        self.identifier += 1
        message = {'jsonrpc': '2.0', 'method': method, 'params': params}
        if not notify:
            message['id'] = self.identifier
        request = Request(self.endpoint, data=json.dumps(message, allow_nan=False).encode(),
            headers={'Authorization': 'Bearer '+self.bearer, 'Content-Type': 'application/json',
                     'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-11-25'})
        try:
            with self.opener.open(request, timeout=40) as response:
                if notify:
                    if response.status not in (200, 202):
                        raise ValueError('MCP initialization notification failed')
                    return None
                if response.headers.get_content_type() != 'application/json':
                    raise ValueError('The QSPQuest endpoint must use its supported JSON HTTP response mode')
                raw = response.read(MAX_RESPONSE+1)
        except (HTTPError, URLError) as error:
            # Do not reflect upstream bodies, URLs or authorization material.
            status = getattr(error, 'code', None)
            if isinstance(error, HTTPError):
                error.close()
            raise RuntimeError('Managed MCP request rejected or unavailable'+(f' (HTTP {status})' if status else '')) from None
        if len(raw) > MAX_RESPONSE:
            raise ValueError('MCP response exceeds the bounded byte limit')
        value = _json(raw)
        if not isinstance(value, dict) or value.get('id') != self.identifier:
            raise ValueError('MCP response identity differs')
        if value.get('error') is not None:
            data = value['error'].get('data') if isinstance(value['error'], dict) else None
            data = data if isinstance(data, dict) else {}
            nested = data.get('error')
            nested = nested if isinstance(nested, dict) else {}
            code = nested.get('code') or data.get('code') or 'resource_failed'
            raise RuntimeError('Managed MCP resource failed: '+str(code))
        return value['result']

    def read_json(self, uri):
        result = self.rpc('resources/read', {'uri': uri})
        contents = result.get('contents') if isinstance(result, dict) else None
        if not isinstance(contents, list) or len(contents) != 1 or not isinstance(contents[0], dict) or contents[0].get('uri') != uri or contents[0].get('mimeType') != 'application/json' or not isinstance(contents[0].get('text'), str):
            raise ValueError('Managed artifact resource must return its exact bounded JSON object')
        value = _json(contents[0]['text'])
        if not isinstance(value, dict):
            raise ValueError('Managed artifact resource is not a JSON object')
        return value


def _integer(value, maximum):
    if type(value) is not int or not 0 <= value <= maximum:
        raise ValueError('Artifact counts and times require bounded nonnegative integers')
    return value


def descriptor(value, expected_uri=None):
    keys = {'schema', 'uri', 'artifact_id', 'organization_id', 'project_id', 'sha256', 'bytes', 'media_type', 'result_schema',
            'created_at_unix_seconds', 'expires_at_unix_seconds', 'retention_seconds', 'chunk_bytes', 'chunk_count', 'chunk_uri_prefix', 'instructions'}
    if not isinstance(value, dict) or set(value) != keys or value['schema'] != 'qspquest.managed-result-artifact/v1':
        raise ValueError('Unsupported or incomplete retained artifact descriptor')
    for key in ('artifact_id', 'organization_id', 'project_id'):
        if not isinstance(value[key], str) or not re.fullmatch('[A-Za-z0-9_-]{1,128}', value[key]):
            raise ValueError('Artifact scope identity differs')
    if not isinstance(value['sha256'], str) or not re.fullmatch('[0-9a-f]{64}', value['sha256']):
        raise ValueError('Artifact requires an exact SHA-256')
    for key in ('media_type', 'instructions'):
        if not isinstance(value[key], str) or not value[key]:
            raise ValueError('Artifact media type and interpretation instructions are required')
    if len(value['media_type']) > 200 or value['result_schema'] is not None and (not isinstance(value['result_schema'], str) or not 1 <= len(value['result_schema']) <= 200):
        raise ValueError('Artifact media/result schema declaration differs')
    size = _integer(value['bytes'], MAX_BYTES)
    chunk_bytes = _integer(value['chunk_bytes'], CHUNK_BYTES)
    count = _integer(value['chunk_count'], (MAX_BYTES+CHUNK_BYTES-1)//CHUNK_BYTES)
    if chunk_bytes != CHUNK_BYTES or count != (size+CHUNK_BYTES-1)//CHUNK_BYTES:
        raise ValueError('Artifact chunk count/size differs from the complete byte length')
    uri = PREFIX+value['artifact_id']+'/'+value['sha256']
    if value['uri'] != uri or expected_uri is not None and uri != expected_uri or value['chunk_uri_prefix'] != uri+'/chunks/':
        raise ValueError('Artifact URI is not bound to its exact identity and digest')
    created = _integer(value['created_at_unix_seconds'], 2**53-1)
    expires = _integer(value['expires_at_unix_seconds'], 2**53-1)
    retention = _integer(value['retention_seconds'], 604_800)
    if retention == 0 or created+retention != expires or created > time.time():
        raise ValueError('Artifact retention declaration differs')
    if time.time() >= expires:
        raise ValueError('managed_artifact_expired')
    return value


def _save(path, value):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def download(read_json, manifest, out):
    """Stream verified chunks to a new private directory; retain labeled failures."""
    manifest = descriptor(manifest)
    out = Path(out)
    out.mkdir(mode=0o700, parents=False)
    _save(out/'descriptor.json', manifest)
    total, digest, chunks = 0, hashlib.sha256(), 0
    try:
        confirmed = descriptor(read_json(manifest['uri']), manifest['uri'])
        if confirmed != manifest:
            raise ValueError('Artifact descriptor changed between discovery and retrieval')
        with os.fdopen(os.open(out/'artifact.partial', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb') as stream:
            for index in range(manifest['chunk_count']):
                if time.time() >= manifest['expires_at_unix_seconds']:
                    raise ValueError('managed_artifact_expired')
                chunk = read_json(manifest['chunk_uri_prefix']+str(index))
                expected = min(CHUNK_BYTES, manifest['bytes']-total)
                keys = {'schema', 'artifact_uri', 'artifact_sha256', 'index', 'offset', 'bytes', 'sha256', 'encoding', 'data'}
                if not isinstance(chunk, dict) or set(chunk) != keys or chunk['schema'] != 'qspquest.managed-artifact-chunk/v1' or chunk['artifact_uri'] != manifest['uri'] or chunk['artifact_sha256'] != manifest['sha256'] or chunk['encoding'] != 'base64':
                    raise ValueError('Artifact chunk schema, identity or encoding differs')
                if _integer(chunk['index'], manifest['chunk_count']) != index or _integer(chunk['offset'], MAX_BYTES) != total or _integer(chunk['bytes'], CHUNK_BYTES) != expected:
                    raise ValueError('Artifact chunk ordering or byte range differs')
                if not isinstance(chunk['data'], str) or len(chunk['data']) > (CHUNK_BYTES+2)//3*4:
                    raise ValueError('Artifact chunk exceeds its encoded byte budget')
                raw = base64.b64decode(chunk['data'], validate=True)
                if len(raw) != expected or hashlib.sha256(raw).hexdigest() != chunk['sha256']:
                    raise ValueError('Artifact chunk SHA-256 or byte length differs')
                stream.write(raw)
                digest.update(raw)
                total += len(raw)
                chunks += 1
        if total != manifest['bytes'] or digest.hexdigest() != manifest['sha256'] or time.time() >= manifest['expires_at_unix_seconds']:
            raise ValueError('Artifact full SHA-256, total bytes or retention differs')
        (out/'artifact.partial').rename(out/'artifact.bin')
        report = {'status': 'passed', 'bytes': total, 'sha256': digest.hexdigest(), 'chunks': chunks,
                  'artifact_uri': manifest['uri'], 'scientific_qualification': 'not_assessed',
                  'scope': 'Complete-byte transport integrity only; inspect the retained result and its scientific evidence separately'}
        _save(out/'checks.json', report)
        return report
    except (ValueError, OSError, RuntimeError, KeyError, TypeError) as error:
        _save(out/'checks.json', {'status': 'failed', 'bytes_retained': total, 'chunks_retained': chunks, 'error': str(error),
                                 'scientific_qualification': 'not_assessed', 'partial_bytes_are_complete': False})
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mcp-url', required=True)
    parser.add_argument('--artifact-id', required=True)
    parser.add_argument('--out', type=Path, required=True, help='New private directory; parent must exist')
    args = parser.parse_args()
    if args.out.exists() or args.out.is_symlink():
        parser.error('Choose a new private output directory; existing results are never replaced')
    try:
        client = Client(args.mcp_url, os.environ.get('QSPQUEST_MCP_BEARER', ''))
        client.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                                 'clientInfo': {'name': 'qspquest-artifact-download', 'version': '1'}})
        client.rpc('notifications/initialized', {}, True)
        response = client.rpc('tools/call', {'name': 'get_artifact', 'arguments': {'artifact_id': args.artifact_id}})
        payload = response.get('structuredContent') if isinstance(response, dict) else None
        contract = payload.get('contract') if isinstance(payload, dict) else None
        if not isinstance(contract, dict) or contract.get('status') != 'success':
            raise ValueError('Authorized artifact metadata retrieval failed')
        retrieval = payload.get('artifact_retrieval', {})
        if not isinstance(retrieval, dict):
            raise ValueError('Authorized artifact metadata lacks its retrieval state')
        if retrieval.get('state') != 'available':
            raise ValueError('Artifact bytes are unavailable: '+str(retrieval.get('code', 'unsupported_retention_contract')))
        manifest = descriptor(retrieval.get('descriptor'))
        if payload.get('full_result_resource') != manifest['uri'] or manifest['artifact_id'] != args.artifact_id:
            raise ValueError('Artifact discovery identity differs')
        print(json.dumps(download(client.read_json, manifest, args.out), indent=2))
        return 0
    except (ValueError, RuntimeError, OSError, KeyError, TypeError) as error:
        print('Artifact retrieval failed: '+str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
