"""Discover and execute the bundled synthetic model using the MCP stdio API."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

from stdio_rpc import StdioRPC
try:
    from result_bundle import check_result, write_bundle
except ModuleNotFoundError:
    from qspquest.result_bundle import check_result, write_bundle


def first_run(directory, binary=None, output=None):
    directory = Path(directory).resolve()
    binary = Path(binary).resolve() if binary else directory / ('qspquest-mcp.exe' if os.name == 'nt' else 'qspquest-mcp')
    if output is not None and (Path(output).exists() or Path(output).is_symlink()):
        raise ValueError('Choose a new output directory: ' + str(output))
    env = {k: v for k, v in os.environ.items() if not k.startswith('QSPQUEST_')}
    client = StdioRPC([str(binary), '--profile', 'local', '--library-dir', str(directory / 'models')],
                      cwd=directory, env=env)
    cases = []
    try:
        client.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                                 'clientInfo': {'name': 'QSPQuest first run', 'version': '1'}}, timeout=120)
        client.rpc('notifications/initialized', {}, True)
        tools = {v['name']: v for v in client.rpc('tools/list', {})['tools']}
        caps = json.loads(''.join(v['text'] for v in client.rpc('resources/read',
                              {'uri': 'qspquest://capabilities'})['contents']))
        if caps['deployment']['profile'] != 'local' or caps['deployment']['mode'] != 'local_stdio':
            raise ValueError('Expected the local stdio profile')
        if not all(v.get('outputSchema') and v.get('annotations') for v in tools.values()):
            raise ValueError('Tool output contracts or annotations are missing')

        def tool(name, arguments, expected='success'):
            response = client.rpc('tools/call', {'name': name, 'arguments': arguments})
            value = response.get('structuredContent', {})
            if (value.get('contract', {}).get('status') != expected
                    or bool(response.get('isError')) != (expected != 'success')):
                raise ValueError(f'{name} failed: {value}')
            cases.append({'tool': name, 'status': expected})
            return value

        found = tool('search_models', {'query': 'synthetic_one_compartment'})
        if not any(v['id'] == 'synthetic_one_compartment' for v in found['models']):
            raise ValueError('The bundled model was not discovered')
        card = tool('get_model', {'model_id': 'synthetic_one_compartment', 'detail': 'full'})
        if (card['metadata']['id'] != 'synthetic_one_compartment'
                or card['metadata'].get('execution_available') is not True):
            raise ValueError('The discovered model is unavailable')
        scenario = tool('get_scenario_request', {'model_id': 'synthetic_one_compartment', 'scenario_id': 'default'})
        request = json.loads((directory / 'models/synthetic-one-compartment.request.json').read_bytes())
        if scenario['request'] != request:
            raise ValueError('The effective scenario differs from the bundled request')
        source = (directory / 'models/synthetic-one-compartment.qsp').read_text(encoding='utf-8')
        tool('validate_model', {'source': source})
        tool('get_model', {'model_id': 'missing-first-run-model'}, 'request_failure')
        arguments = {'model': {'library': {'model_id': 'synthetic_one_compartment'}},
                     'request': request, 'outputs': ['cp'], 'detail': 'full'}
        planned = tool('plan_run', {'operation': 'simulate', 'arguments': arguments})
        plan = planned.get('plan')
        if plan is None:
            uri = planned.get('full_plan_resource')
            if not uri or uri not in planned['contract']['resources']:
                raise ValueError('The preparation plan is incomplete')
            def read_plan(uri):
                return ''.join(part['text'] for part in client.rpc('resources/read', {'uri': uri})['contents'])
            plan = json.loads(read_plan(uri))
            if plan.get('kind') == 'utf8_chunk_manifest':
                chunks = plan.get('chunks')
                if not isinstance(chunks, list) or not 1 <= len(chunks) <= 256:
                    raise ValueError('The preparation plan exceeds this example chunk budget')
                text = ''.join(read_plan(chunk) for chunk in chunks)
                if len(text.encode('utf-8')) > 4 * 1024 * 1024 or hashlib.sha256(text.encode('utf-8')).hexdigest() != plan['sha256']:
                    raise ValueError('Preparation plan size or checksum differs')
                plan = json.loads(text)
        if (plan.get('schema') != 'qspquest.mcp-request-plan/v0.1'
                or plan.get('native_preparation', {}).get('kind') != 'simulate'
                or not isinstance(plan.get('plan_hash'), str) or not plan['plan_hash'].startswith('sha256:')
                or plan['plan_hash'] != planned.get('plan_hash')
                or plan.get('context', {}).get('effective_request_hash') != planned.get('effective_request_hash')
                or plan.get('qualification', {}).get('current_engine_and_request') != 'not_assessed'):
            raise ValueError('Inspect a complete simulation preparation plan with explicit qualification limits')
        result = tool('simulate', dict(arguments, expected_plan_hash=plan['plan_hash']))
        completeness = result['contract']['completeness']
        if completeness == 'resource_linked':
            uri = result['full_result_resource']
            if uri not in result['contract']['resources']:
                raise ValueError('The full result resource is missing from its contract')
            def read(uri):
                return ''.join(v['text'] for v in client.rpc('resources/read', {'uri': uri})['contents'])
            text = read(uri)
            resource = json.loads(text)
            if resource.get('schema') == 'qspquest.result-artifact/v0.1':
                count = resource.get('chunk_count')
                if type(count) is not int or not 1 <= count <= 256 or resource.get('bytes', 0) > 16 * 1024 * 1024:
                    raise ValueError('Complete resource exceeds this example budget')
                text = ''.join(read(resource['chunk_uri_prefix'] + str(index)) for index in range(count))
                if len(text.encode('utf-8')) != resource['bytes'] or hashlib.sha256(text.encode('utf-8')).hexdigest() != resource['sha256']:
                    raise ValueError('Full result byte count or digest differs')
                resource = json.loads(text)
            elif resource.get('kind') == 'utf8_chunk_manifest':
                chunks = resource['chunks']
                if not isinstance(chunks, list) or not 1 <= len(chunks) <= 256:
                    raise ValueError('Complete resource exceeds this example chunk budget')
                text = ''.join(read(chunk) for chunk in chunks)
                if len(text.encode('utf-8')) > 16 * 1024 * 1024 or hashlib.sha256(text.encode('utf-8')).hexdigest() != resource['sha256']:
                    raise ValueError('Full result chunk size or digest differs')
                resource = json.loads(text)
            if resource['identity'] != result['identity'] or resource['effective_request_hash'] != result['effective_request_hash']:
                raise ValueError('Full result identity differs from the simulation response')
            result = resource
        elif completeness != 'inline_complete':
            raise ValueError('The simulation did not return a complete result')
        checks = check_result(result)
        if result.get('effective_request_hash') != plan['context']['effective_request_hash']:
            raise ValueError('Execution input identity differs from the inspected preparation plan')
        if (directory / 'models/synthetic-one-compartment.qsp').read_text(encoding='utf-8') != source or json.loads((directory / 'models/synthetic-one-compartment.request.json').read_bytes()) != request:
            raise ValueError('Bundled input files changed during execution')
        bundle = write_bundle(output, result, source, request, producer='mcp') if output is not None else None
        cp = next(column['values'] for column in result['outputs'] if column['name'] == 'cp')
        return {'status': 'passed', 'capabilities': caps, 'identity': result['identity'],
                'cases': cases, 'times': result['times'], 'concentrations_mg_per_l': cp,
                'response_completeness': completeness, 'full_result_retrieved': True,
                'analytic_absolute_limit': checks['analytic_absolute_limit_mg_L'],
                'max_absolute_error': checks['max_absolute_error_mg_L'],
                'plan': plan, 'result': result, 'bundle': bundle,
                'qualification': checks['scope']}
    finally:
        client.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=Path(__file__).resolve().parent,
                        help='Extracted MCP directory containing models/')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--out', type=Path, default=Path('agent-results'))
    args = parser.parse_args()
    try:
        print(json.dumps(first_run(args.directory, args.binary, args.out), indent=2, allow_nan=False))
    except (ValueError, RuntimeError, TimeoutError, OSError, KeyError, TypeError) as error:
        print(f'First run failed: {error}', file=sys.stderr)
        raise SystemExit(1)


if __name__ == '__main__':
    main()
