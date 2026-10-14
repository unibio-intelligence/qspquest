"""Bounded, portable line-delimited MCP client for line-delimited MCP requests."""
import json
import hashlib
import queue
import subprocess
import threading
import time


class StdioRPC:
    def __init__(self, command, *, env=None, cwd=None, max_line_bytes=16 * 1024 * 1024):
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, env=env, cwd=cwd)
        self.responses = queue.Queue(maxsize=4)
        self.diagnostics = bytearray()
        self.lock = threading.Lock()
        self.identifier = 0
        self.max_line_bytes = max_line_bytes
        self.readers = [threading.Thread(target=self._stdout, daemon=True),
                        threading.Thread(target=self._stderr, daemon=True)]
        for reader in self.readers:
            reader.start()

    def _stdout(self):
        try:
            while True:
                line = self.process.stdout.readline(self.max_line_bytes + 1)
                if not line:
                    self.responses.put(EOFError('MCP stdout closed'))
                    return
                if len(line) > self.max_line_bytes or not line.endswith(b'\n'):
                    raise ValueError('MCP response exceeds the line limit or is incomplete')
                self.responses.put(json.loads(line))
        except Exception as error:
            self.responses.put(error)

    def _stderr(self):
        while True:
            chunk = self.process.stderr.read1(4096)
            if not chunk:
                return
            with self.lock:
                self.diagnostics.extend(chunk)
                del self.diagnostics[:-65536]

    def diagnostic_text(self):
        with self.lock:
            return bytes(self.diagnostics).decode('utf-8', errors='replace')

    def read_json(self, uri, *, max_bytes=16 * 1024 * 1024):
        """Read a complete JSON resource, including bounded, digest-checked chunks."""
        def read_text(location):
            contents = self.rpc('resources/read', {'uri': location})['contents']
            if not contents or any(not isinstance(part.get('text'), str) for part in contents):
                raise ValueError('Expected UTF-8 text resource contents')
            raw = ''.join(part['text'] for part in contents).encode('utf-8')
            if len(raw) > max_bytes:
                raise ValueError('Resource exceeds this client byte budget')
            return raw
        raw = read_text(uri)
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise ValueError('Expected a complete JSON object')
        if value.get('kind') == 'utf8_chunk_manifest':
            locations = value.get('chunks')
            if not isinstance(locations, list) or not 1 <= len(locations) <= 256:
                raise ValueError('Resource exceeds this client chunk budget')
        elif value.get('schema') == 'qspquest.result-artifact/v0.1':
            count = value.get('chunk_count')
            if type(count) is not int or not 1 <= count <= 256:
                raise ValueError('Artifact exceeds this client chunk budget')
            locations = [value['chunk_uri_prefix'] + str(index) for index in range(count)]
        else:
            return value
        chunks, size = [], 0
        for location in locations:
            chunk = read_text(location)
            size += len(chunk)
            if size > max_bytes:
                raise ValueError('Complete resource exceeds this client byte budget')
            chunks.append(chunk)
        raw = b''.join(chunks)
        if hashlib.sha256(raw).hexdigest() != value.get('sha256') or ('bytes' in value and value['bytes'] != len(raw)):
            raise ValueError('Complete resource byte count or SHA-256 differs')
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise ValueError('Expected a complete JSON object')
        return value

    def rpc(self, method, params, notify=False, *, timeout=60):
        self.identifier += 1
        message = {'jsonrpc': '2.0', 'method': method, 'params': params}
        if not notify:
            message['id'] = self.identifier
        try:
            self.process.stdin.write((json.dumps(message, ensure_ascii=False) + '\n').encode('utf-8'))
            self.process.stdin.flush()
        except (BrokenPipeError, OSError) as error:
            raise RuntimeError('MCP stdin closed: ' + self.diagnostic_text()) from error
        if notify:
            return
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f'MCP {method} timeout: ' + self.diagnostic_text())
            try:
                response = self.responses.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError(f'MCP {method} timeout: ' + self.diagnostic_text()) from error
            if (isinstance(response, dict) and response.get('jsonrpc') == '2.0'
                    and 'id' not in response and isinstance(response.get('method'), str)):
                continue
            break
        if isinstance(response, Exception):
            # Give an exited child's diagnostic reader time to collect its tail.
            self.readers[1].join(timeout=0.1)
            raise RuntimeError(f'MCP response failed: {response}: ' + self.diagnostic_text()) from response
        if (not isinstance(response, dict) or response.get('jsonrpc') != '2.0'
                or response.get('id') != self.identifier or 'error' in response
                or 'result' not in response):
            raise ValueError(f'Unexpected MCP response: {response}')
        return response['result']

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)
        for reader in self.readers:
            reader.join(timeout=1)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            stream.close()

