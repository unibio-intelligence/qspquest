#!/usr/bin/env python3
"""Check the packaged synthetic first run over HTTP or container stdio."""
import argparse
import json
import math
import queue
import subprocess
import threading
import time
from urllib.request import Request, build_opener, ProxyHandler



def check_simulation(result):
    """Require the complete requested synthetic result before claiming success."""
    def finite_number(value):
        return type(value) in (int, float) and math.isfinite(value)

    if (not isinstance(result, dict)
            or result.get('schema') != 'qspquest.result/v0.1'
            or result.get('model_id') != 'synthetic_one_compartment'
            or result.get('contract', {}).get('status') != 'success'
            or result.get('error') is not None):
        raise ValueError('Simulation result identity or success status differs')
    times = result.get('times')
    if (not isinstance(times, list) or not all(finite_number(t) for t in times)
            or times != [0, 2, 8, 24] or result.get('time_unit') != 'h'
            or result.get('sides') != ['post'] * 4):
        raise ValueError('Simulation times, time unit or event sides differ')
    outputs = result.get('outputs')
    if (not isinstance(outputs, list) or len(outputs) != 1
            or not isinstance(outputs[0], dict) or outputs[0].get('name') != 'cp'
            or outputs[0].get('unit') != 'mg/L'):
        raise ValueError('Simulation must return exactly one cp output in mg/L')
    column = outputs[0]
    values = column.get('values')
    if (not isinstance(values, list) or len(values) != len(times)
            or not all(finite_number(value) for value in values)
            or any(abs(value - 2 * math.exp(-.1 * t)) >= 2e-7
                   for t, value in zip(times, values))):
        raise ValueError('Simulation concentrations are incomplete, nonfinite or incorrect')
    return column


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    transport = parser.add_mutually_exclusive_group(required=True)
    transport.add_argument('--url', help='Loopback server URL, optionally ending in /mcp')
    transport.add_argument('--stdio-image', help='Loaded QSPQuest MCP Docker image')
    args = parser.parse_args()
    process = None
    replies = queue.Queue()
    sequence = 0
    opener = build_opener(ProxyHandler({}))
    if args.stdio_image:
        process = subprocess.Popen([
            'docker', 'run', '--rm', '-i', '--network', 'none', '--read-only',
            '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
            args.stdio_image, '--profile', 'local'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

        def receive():
            try:
                for line in process.stdout:
                    replies.put(json.loads(line))
                replies.put(RuntimeError('Container stopped before returning a response'))
            except Exception as error:
                replies.put(error)
        threading.Thread(target=receive, daemon=True).start()
    else:
        base = args.url.rstrip('/').removesuffix('/mcp')
        deadline = time.monotonic() + 60
        while True:
            try:
                with opener.open(base + '/healthz', timeout=2) as response:
                    if response.status == 200:
                        break
            except OSError:
                if time.monotonic() >= deadline:
                    raise RuntimeError('Server not ready: inspect docker logs and the port mapping')
                time.sleep(.2)

    def rpc(method, params, notification=False):
        nonlocal sequence
        sequence += 1
        message = {'jsonrpc': '2.0', 'method': method, 'params': params}
        if not notification:
            message['id'] = sequence
        raw = json.dumps(message)
        if process:
            process.stdin.write(raw + '\n')
            process.stdin.flush()
            if notification:
                return
            reply = replies.get(timeout=60)
            if isinstance(reply, Exception):
                raise reply
        else:
            request = Request(base + '/mcp', data=raw.encode(), headers={
                'Content-Type': 'application/json',
                'Accept': 'application/json, text/event-stream',
                'MCP-Protocol-Version': '2025-11-25'})
            with opener.open(request, timeout=60) as response:
                encoded = response.read(2_000_001)
                if notification:
                    return
                assert len(encoded) <= 2_000_000, 'Response exceeded client budget'
                reply = json.loads(encoded)
        assert reply.get('id') == sequence and 'error' not in reply, reply
        return reply['result']

    def call(name, arguments):
        reply = rpc('tools/call', {'name': name, 'arguments': arguments})
        assert not reply.get('isError'), reply
        value = reply['structuredContent']
        assert value['contract']['status'] == 'success', value
        return value

    try:
        rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                          'clientInfo': {'name': 'QSPQuest quickstart', 'version': '1.0'}})
        rpc('notifications/initialized', {}, notification=True)
        names = {tool['name'] for tool in rpc('tools/list', {})['tools']}
        assert {'search_models', 'get_model', 'simulate'} <= names
        models = call('search_models', {'query': 'synthetic_one_compartment'})
        assert any(model['id'] == 'synthetic_one_compartment' for model in models['models'])
        call('get_model', {'model_id': 'synthetic_one_compartment', 'detail': 'full'})
        result = call('simulate', {
            'model': {'library': {'model_id': 'synthetic_one_compartment'}},
            'request': {
                'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf',
                'budgets': {'solver_callbacks': 1000000, 'events': 100,
                            'output_values': 10000},
                'regimen': {
                    'end': {'value': 24, 'unit': 'h'}, 'samples': [],
                    'observations': [{'time': {'value': t, 'unit': 'h'}, 'side': 'post'}
                                     for t in [0, 2, 8, 24]],
                    'administrations': [{'target': 'central',
                        'amount': {'value': 6, 'unit': 'mg'},
                        'time': {'value': 0, 'unit': 'h'},
                        'delivery': {'kind': 'bolus'}, 'bioavailability': 1}],
                    'rtol': 1e-10, 'atol': 1e-12}},
            'outputs': ['cp']})
        column = check_simulation(result)
        print(json.dumps({'transport': 'stdio' if process else 'HTTP',
                          'model': 'synthetic_one_compartment', 'time_unit': 'h',
                          'times': result['times'], 'unit': column['unit'],
                          'concentrations': column['values'], 'checks_passed': True}, indent=2))
    finally:
        if process:
            process.stdin.close()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=5)


if __name__ == '__main__':
    main()
