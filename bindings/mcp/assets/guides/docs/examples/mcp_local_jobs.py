"""Run, optionally cancel/resume, and verify two synthetic offline local work units."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time


class Client:
    def __init__(self, binary, library, jobs, artifacts):
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(('QSPQUEST_LIBRARY_', 'QSPQUEST_MCP_', 'QSPQUEST_MANAGED_'))}
        self.process = subprocess.Popen([str(binary), '--profile', 'local', '--library-dir', str(library),
            '--jobs-dir', str(jobs), '--artifact-dir', str(artifacts), '--concurrency', '1'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding='utf-8', bufsize=1, env=environment)
        self.responses = queue.Queue()
        self.diagnostics = []
        self.identifier = 0
        def receive():
            try:
                for line in self.process.stdout:
                    if len(line.encode()) > 16 * 1024 * 1024:
                        raise RuntimeError('Response exceeds the example line budget')
                    self.responses.put(json.loads(line))
                self.responses.put(RuntimeError('Server stopped; inspect startup diagnostics'))
            except Exception as error:
                self.responses.put(error)
        def diagnostics():
            for line in self.process.stderr:
                if len(self.diagnostics) < 30:
                    self.diagnostics.append(line.rstrip())
        threading.Thread(target=receive, daemon=True).start()
        threading.Thread(target=diagnostics, daemon=True).start()
        self.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                               'clientInfo': {'name': 'qspquest-local-job-example', 'version': '1'}})
        self.rpc('notifications/initialized', {}, notification=True)

    def rpc(self, method, params, notification=False):
        self.identifier += 1
        request = {'jsonrpc': '2.0', 'method': method, 'params': params}
        if not notification:
            request['id'] = self.identifier
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        if notification:
            return None
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            reply = self.responses.get(timeout=max(0.01, deadline - time.monotonic()))
            if isinstance(reply, Exception):
                raise RuntimeError(str(reply) + ': ' + '; '.join(self.diagnostics))
            if reply.get('id') != self.identifier:
                continue
            if 'error' in reply:
                raise RuntimeError(reply['error'])
            return reply['result']
        raise RuntimeError('MCP response timed out')

    def tool(self, name, arguments):
        result = self.rpc('tools/call', {'name': name, 'arguments': arguments})
        value = result['structuredContent']
        if result.get('isError'):
            raise RuntimeError(value)
        return value

    def read(self, uri):
        return ''.join(part['text'] for part in self.rpc('resources/read', {'uri': uri})['contents'])

    def full(self, descriptor):
        manifest = json.loads(self.read(descriptor['uri']))
        if manifest != descriptor:
            raise RuntimeError('Artifact manifest identity changed')
        raw = ''.join(self.read(manifest['chunk_uri_prefix'] + str(index))
                      for index in range(manifest['chunk_count'])).encode('utf-8')
        if len(raw) != manifest['bytes'] or hashlib.sha256(raw).hexdigest() != manifest['sha256']:
            raise RuntimeError('Complete artifact byte count or digest differs')
        return json.loads(raw)

    def close(self):
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


def wait(client, identifier):
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        job = client.tool('get_local_job', {'job_id': identifier})['local_job']
        if job['state'] not in ('queued', 'running', 'cancel_requested'):
            return job
        time.sleep(0.1)
    raise RuntimeError('Job did not reach a terminal state within the example budget')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--cancel', action='store_true')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    source = (Path(__file__).resolve().parents[2] / 'conformance/models/synthetic-one-compartment.qsp').read_text()
    with tempfile.TemporaryDirectory(prefix='qspquest-local-jobs-') as scratch:
        root = Path(scratch)
        library, jobs, artifacts = root / 'library', root / 'jobs', root / 'artifacts'
        library.mkdir(mode=0o700)
        (library / 'synthetic.qsp').write_text(source)
        work = []
        for dose in (6, 12):
            work.append({'label': f'dose-{dose}-mg', 'operation': 'simulate', 'arguments': {
                'model': {'source': source}, 'detail': 'full', 'outputs': ['cp'],
                'request': {'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf',
                    'budgets': {'solver_callbacks': 1000000, 'events': 100, 'output_values': 100000},
                    'regimen': {'start': {'value': 0, 'unit': 'h'}, 'end': {'value': 30, 'unit': 'h'},
                        'rtol': 1e-10, 'atol': 1e-12, 'samples': [],
                        'observations': [{'time': {'value': hour, 'unit': 'h'}, 'side': 'post'} for hour in range(31)],
                        'administrations': [{'target': 'central', 'amount': {'value': dose, 'unit': 'mg'},
                            'time': {'value': 0, 'unit': 'h'}, 'delivery': {'kind': 'bolus'},
                            'bioavailability': 1}]}}}})
        request = {'idempotency_key': 'demo-1', 'work': work}
        client = Client(binary, library, jobs, artifacts)
        try:
            admitted = client.tool('submit_local_job', request)['local_job']
            identifier = admitted['job_id']
            assert client.tool('submit_local_job', request)['local_job']['job_id'] == identifier
            if args.cancel:
                client.tool('cancel_local_job', {'job_id': identifier})
            job = wait(client, identifier)
            if job['state'] == 'cancelled':
                client.tool('resume_local_job', {'job_id': identifier})
                job = wait(client, identifier)
            if job['state'] != 'succeeded' or len(job['completed']) != 2 or job['scientific_result']:
                raise RuntimeError(job)
            digests = []
            for dose, unit in zip((6, 12), job['completed']):
                result = client.full(unit['artifact'])
                if result['contract']['status'] != 'success' or unit['scientific_status'] != 'success':
                    raise RuntimeError('Synthetic simulation has no successful scientific payload')
                columns = {column['name']: column for column in result['outputs']}
                concentration = columns['cp']
                if concentration['unit'] != 'mg/L' or result['time_unit'] != 'h':
                    raise RuntimeError('Synthetic output units differ')
                if len(result['times']) != 31 or len(concentration['values']) != 31:
                    raise RuntimeError('Complete result grid is missing')
                if result['times'] != list(range(31)) or result['sides'] != ['post'] * 31:
                    raise RuntimeError('The explicit hourly post-dose observation plan differs')
                for hour, side, value in zip(result['times'], result['sides'], concentration['values']):
                    expected = dose / 3 * math.exp(-0.1 * hour) if side == 'post' or hour > 0 else 0
                    if not math.isclose(value, expected, rel_tol=5e-5, abs_tol=5e-7):
                        raise RuntimeError('Independent analytical concentration differs')
                digests.append(unit['artifact']['sha256'])
        finally:
            client.close()
        client = Client(binary, library, jobs, artifacts)
        try:
            resumed = client.tool('resume_local_job', {'job_id': identifier})['local_job']
            assert resumed['state'] == 'succeeded' and resumed['dispatched_units'] == job['dispatched_units']
            assert [unit['artifact']['sha256'] for unit in resumed['completed']] == digests
            assert client.tool('submit_local_job', request)['local_job']['job_id'] == identifier
            for unit in resumed['completed']:
                client.full(unit['artifact'])
            print('Verified two synthetic dose results, full artifacts, stable retry key and restart without replay.')
        finally:
            client.close()


if __name__ == '__main__':
    main()
