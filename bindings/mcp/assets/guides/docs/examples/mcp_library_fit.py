"""Discover, plan and execute an exact archived synthetic fitting workflow locally."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
from stdio_rpc import StdioRPC


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--workflow', help='Exact v1 workflow ID; required when several passed workflows exist')
    parser.add_argument('--out', type=Path, default=Path('library-fit-results'))
    args = parser.parse_args()
    if args.out.exists() or args.out.is_symlink(): parser.error('Choose a new output directory')
    env = {key: value for key, value in os.environ.items() if not key.startswith('QSPQUEST_')}
    client = StdioRPC([str(args.binary.resolve()), '--profile', 'local', '--library-dir', str(args.library.resolve())], env=env)
    try:
        client.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                                'clientInfo': {'name': 'qspquest-library-fitting-example', 'version': '1'}})
        client.rpc('notifications/initialized', {}, True)
        def tool(name, arguments, *, retain_failure=False):
            response = client.rpc('tools/call', {'name': name, 'arguments': arguments})
            value = response.get('structuredContent')
            if not isinstance(value, dict) or not value.get('contract'):
                raise ValueError('A complete typed tool response is required')
            if response.get('isError') and not retain_failure:
                raise ValueError(name + ': ' + json.dumps(value))
            return value
        def complete(value, key):
            uri = value.get(key)
            if uri:
                if uri not in value['contract'].get('resources', []):
                    raise ValueError('Complete resource is not declared by its contract')
                full = client.read_json(uri)
                for binding in ('identity', 'effective_request_hash', 'plan_hash'):
                    if binding in value and binding in full and value[binding] != full[binding]:
                        raise ValueError('Complete resource identity differs: ' + binding)
                return full
            if value['contract'].get('completeness') != 'inline_complete':
                raise ValueError('A preview or routed operation is not a complete calculation')
            return value
        card = complete(tool('get_model', {'model_id': args.model, 'version': args.version, 'detail': 'full'}), 'full_model_details_resource')
        metadata, declaration = card['metadata'], card['fitting']
        if metadata.get('fit_ready') is not True or metadata.get('fitting_supported') is not True:
            raise ValueError('This exact library version is not fit ready. Read its fitting reason and choose a declared ready workflow; simulation availability does not imply fitting support')
        model = {'library': {'model_id': args.model, 'version': args.version}}
        if declaration.get('schema') == 'qspquest.library-fitting/v1':
            workflows = [workflow for workflow in declaration['workflows'] if workflow['qualification']['status'] == 'passed'
                         and (args.workflow is None or workflow['id'] == args.workflow)]
            if len(workflows) != 1: raise ValueError('Select one exact passed workflow ID from get_model.fitting.workflows')
            workflow = workflows[0]
            operation = workflow['operation']
            arguments = {'model': model, 'fit_request': workflow['native_request']}
            expected = {role['native_name']: (role['synthetic_truth'], role['recovery_absolute_tolerance'], role['unit']) for role in workflow['parameters']}
        elif declaration.get('schema') == 'qspquest.mcp-fitting-contract/v0.1':
            if args.workflow: raise ValueError('The legacy scalar contract has no v1 workflow ID')
            fixture = client.read_json('qspquest://examples/fit/library-scalar')
            operation, arguments = fixture['tool'], fixture['arguments']
            if operation != 'fit_scalar' or arguments['model'] != model:
                raise ValueError('This library version has no bundled legacy fitting example; supply its separately documented native request')
            expected = {declaration['parameter']: (declaration['qualification']['parameter_truth'],
                        declaration['qualification']['absolute_recovery_tolerance'], declaration['lower']['unit'])}
        else:
            raise ValueError('Unsupported fitting contract schema')
        planned = tool('plan_run', {'operation': operation, 'arguments': arguments})
        if planned.get('plan') is not None:
            plan = planned['plan']
        else:
            plan = complete(planned, 'full_plan_resource')
        if plan.get('plan_hash') != planned.get('plan_hash') or plan.get('context', {}).get('operation') != operation:
            raise ValueError('The complete preparation plan differs from the requested workflow')
        arguments = dict(arguments, expected_plan_hash=plan['plan_hash'])
        response = tool(operation, arguments, retain_failure=True)
        result = complete(response, 'full_result_resource')
        fit = result.get('fit', result)
        status = fit.get('status', response['contract']['status'])
        checks = {}
        for name, (truth, tolerance, unit) in expected.items():
            estimate = fit.get('parameters', {}).get(name, {})
            value = estimate.get('value')
            checks[name] = {'truth': truth, 'unit': unit, 'absolute_limit': tolerance,
                            'estimate': value, 'passed': type(value) in (int, float) and estimate.get('unit') == unit and abs(value - truth) <= tolerance}
        passed = status == 'converged' and response['contract']['status'] == 'success' and all(check['passed'] for check in checks.values())
        report = {'status': 'passed' if passed else 'failed', 'fit_status': status, 'parameter_checks': checks,
                  'projected_gradient_norm': fit.get('projected_gradient_norm'), 'evaluations': fit.get('evaluations'),
                  'iterations': fit.get('iterations'), 'fit_request_hash': fit.get('fit_request_hash'),
                  'scope': 'Exact synthetic recovery fixture only; no source population reproduction, clinical qualification or current-engine scientific qualification'}
        files = {'model-card.json': card, 'arguments.json': arguments, 'plan.json': plan, 'result.json': result, 'checks.json': report}
        encoded = {name: (json.dumps(value, indent=2, allow_nan=False) + '\n').encode('utf-8') for name, value in files.items()}
        args.out.mkdir(mode=0o700, parents=False)
        for name, raw in encoded.items():
            with (args.out / name).open('xb') as stream: stream.write(raw)
        print(json.dumps(dict(report, directory=str(args.out.absolute()),
            files={name: {'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)} for name, raw in encoded.items()}), indent=2, allow_nan=False))
        if not passed: raise SystemExit(1)
    except (ValueError, OSError, RuntimeError, TimeoutError, KeyError, TypeError) as error:
        print('Library fitting failed: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
    finally:
        client.close()


if __name__ == '__main__':
    main()
