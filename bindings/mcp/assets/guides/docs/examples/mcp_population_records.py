"""Plan, check row admission, and fit the shipped synthetic population records."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import sys
from stdio_rpc import StdioRPC


def save(directory, name, value):
    with (directory / name).open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--out', type=Path, default=Path('population-record-results'))
    args = parser.parse_args()
    if args.out.exists() or args.out.is_symlink():
        parser.error('Choose a new output directory; existing results are never replaced')
    try:
        binary = args.binary.resolve(strict=True)
        binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    except OSError as error:
        parser.error('Use the built QSPQuest MCP executable: '+str(error))
    fixture_path = Path(__file__).resolve().parents[2] / 'bindings/mcp/assets/fits/population.json'
    raw = fixture_path.read_bytes()
    fixture = json.loads(raw)
    inputs = fixture['arguments']
    args.out.mkdir(mode=0o700, parents=False)
    save(args.out, 'input.json', fixture)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('QSPQUEST_',))}
    client = None
    try:
        client = StdioRPC([str(binary), '--profile', 'local', '--artifact-dir',
                           str((args.out / 'artifacts').resolve())], env=env)
        client.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                                 'clientInfo': {'name': 'qspquest-population-record-example', 'version': '1'}})
        client.rpc('notifications/initialized', {}, True)
        def tool(name, arguments):
            response = client.rpc('tools/call', {'name': name, 'arguments': arguments}, timeout=180)
            value = response.get('structuredContent')
            if not isinstance(value, dict) or value.get('contract', {}).get('schema') != 'qspquest.mcp-result/v1':
                raise ValueError('Expected the structured result contract')
            if value.get('full_result_resource'):
                value = client.read_json(value['full_result_resource'])
            return value
        malformed = copy.deepcopy(inputs)
        malformed['mapping']['endpoints'] = []
        refusal = tool('plan_run', {'operation': 'fit_population', 'arguments': malformed})
        save(args.out, 'mapping-refusal.json', refusal)
        if refusal['contract']['status'] != 'request_failure' or refusal['error']['code'] != 'dataset' or refusal['error']['input_diagnostic']['field'] != 'mapping.endpoints':
            raise ValueError('Malformed mapping did not produce the expected located admission refusal')
        planned = tool('plan_run', {'operation': 'fit_population', 'arguments': inputs})
        if planned['contract']['status'] != 'success':
            save(args.out, 'plan-failure.json', planned)
            raise ValueError('Static preparation failed; inspect plan-failure.json before changing inputs')
        plan = planned.get('plan')
        if not isinstance(plan, dict) and planned.get('full_plan_resource'):
            retained = client.read_json(planned['full_plan_resource'])
            plan = retained.get('plan', retained)
        if not isinstance(plan, dict) or not isinstance(plan.get('plan_hash'), str):
            raise ValueError('Expected a complete content-bound request plan')
        save(args.out, 'plan.json', plan)
        rows = plan['row_mapping']
        if len(rows) != 24 or sorted({row['subject_index'] for row in rows}) != [0, 1, 2, 3]:
            raise ValueError('The synthetic fixture must retain four subjects and 24 mapped observations')
        run_inputs = dict(inputs, expected_plan_hash=plan['plan_hash'])
        result = tool('fit_population', run_inputs)
        save(args.out, 'result.json', result)
        population = result.get('population', {})
        converged = (result['contract']['status'] == 'success'
                     and population.get('status') == 'converged'
                     and result.get('scientific_status', {}).get('converged') is True)
        recovered = all(abs(population.get('fixed_effects', {}).get(name, {}).get('value', float('inf')) - truth)
                        <= fixture['absolute_tolerance'] for name, truth in fixture['expected_parameters'].items())
        same_rows = result.get('row_mapping') == rows
        same_plan = result.get('plan_hash') == plan['plan_hash']
        if hashlib.sha256(binary.read_bytes()).hexdigest() != binary_sha256:
            raise ValueError('The executable bytes changed during this workflow; retain the results as unqualified')
        report = {'status': 'passed' if converged and recovered and same_rows and same_plan else 'failed',
                  'subjects': 4, 'included_observations': 24,
                  'mapping_refusal_checked': True, 'plan_hash': plan['plan_hash'],
                  'same_row_provenance': same_rows, 'same_execution_plan': same_plan,
                  'converged': converged, 'synthetic_recovery_within_declared_tolerance': recovered,
                  'method': inputs['method'], 'objective_kind': population.get('objective_kind'),
                  'fixture_sha256': hashlib.sha256(raw).hexdigest(),
                  'binary_sha256': binary_sha256,
                  'scientific_qualification': 'not_assessed',
                  'scope': 'One synthetic record-mapping and bounded population teaching workflow; no general bias, coverage, model or clinical qualification'}
        save(args.out, 'checks.json', report)
        print(json.dumps(report, indent=2, allow_nan=False))
        return 0 if report['status'] == 'passed' else 1
    except (ValueError, OSError, RuntimeError, TimeoutError, KeyError, TypeError) as error:
        save(args.out, 'failure.json', {'status': 'failed', 'message': str(error),
                                      'scientific_qualification': 'not_assessed'})
        print('Population record workflow failed: '+str(error), file=sys.stderr)
        return 1
    finally:
        if client is not None:
            client.close()


if __name__ == '__main__':
    raise SystemExit(main())
