"""Fork a library model, extend it by mechanism, and select a regimen against a target.

Every step uses MCP tools only: fork_model, edit_model, get_model with source,
simulate, scan_parameters and compare_regimens. The default model is the Serkland
2025 rituximab–CD19+ B-cell model from the public model library. The added
mechanisms and their values are illustrative assumptions; the selected regimen is
not a dosing recommendation.

    python3 docs/examples/mcp_extend_model.py target/release/qspquest-mcp \
        --library /path/to/qspquest-models
"""
import argparse
import json
import os
from pathlib import Path

from stdio_rpc import StdioRPC

MODEL = 'qspquest_serkland_2025_rituximab_ms_pkpd'
SCENARIO = 'population-typical-1000mg-full-ode'
TARGET = 10.0  # illustrative CD19+ ceiling, cells/mm3, from day 28 to day 730


def q(value, unit):
    return {'value': value, 'unit': unit}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--model', default=MODEL)
    parser.add_argument('--scenario', default=SCENARIO)
    args = parser.parse_args()
    env = {k: v for k, v in os.environ.items() if not k.startswith('QSPQUEST_')}
    client = StdioRPC([str(args.binary.resolve()), '--profile', 'local', '--library-dir',
                       str(args.library.resolve())], env=env)
    try:
        client.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                                  'clientInfo': {'name': 'qspquest-extend-model-example', 'version': '1'}})
        client.rpc('notifications/initialized', {}, True)

        def tool(name, arguments):
            response = client.rpc('tools/call', {'name': name, 'arguments': arguments})
            value = response.get('structuredContent')
            if response.get('isError') or not isinstance(value, dict) or value.get('error'):
                raise ValueError(name + ': ' + json.dumps(value)[:2000])
            return value

        # 1. A readable copy whose stored scenario reproduces the archive exactly.
        fork = tool('fork_model', {'model_id': args.model, 'scenario_id': args.scenario})
        check = fork['scenario']['equivalence']
        assert check['status'] == 'identical', check
        names = {row['native']: row['readable'] for row in fork['name_map']}
        print(f"fork: {len(fork['name_map'])} symbols, {check['compared_values']} values identical")

        # 2. Mechanisms: an IV dosing input, a peripheral tissue compartment and
        #    allometric weight scaling. The source model keeps
        #    rituximab as a concentration (mg/L) in a central volume v_l.
        state, volume, k_linear = names['source_symbol_0'], names['source_symbol_5'], names['source_symbol_6']
        assumption = 'Illustrative assumption for this example; not estimated from data.'
        # The archive encodes its single dose as the starting concentration; regimens
        # are given as doses instead, so the state starts empty.
        start = next(r for r in fork['name_map'] if r['readable'] == state)
        edit = tool('edit_model', {'source': fork['canonical_source'], 'operations': [
            {'op': 'set_initial', 'state': state, 'initial': q(0, start['unit'])},
            {'op': 'add_dosing', 'state': state, 'amount_unit': 'mg', 'scale': f'1 / {volume}'},
            {'op': 'add_peripheral_compartment', 'central': state, 'name': 'rtx_tissue',
             'coordinates': 'concentration', 'central_volume': volume,
             'volume': q(3, 'L'), 'clearance': q(0.3, 'L/d'), 'description': assumption},
            {'op': 'add_covariate', 'name': 'wt', 'unit': 'kg', 'default': q(70, 'kg')},
            # Volume scales with weight and clearance (k * V) with weight^0.75, so the
            # first-order rate constant k scales with weight^-0.25.
            {'op': 'add_covariate_effect', 'parameter': volume, 'covariate': 'wt',
             'reference': q(70, 'kg'), 'form': 'power', 'coefficient': 1.0, 'description': assumption},
            {'op': 'add_covariate_effect', 'parameter': k_linear, 'covariate': 'wt',
             'reference': q(70, 'kg'), 'form': 'power', 'coefficient': -0.25, 'description': assumption},
            {'op': 'add_output', 'name': 'rituximab_tissue_mg_l', 'unit': 'mg/L', 'expression': 'rtx_tissue'},
        ]})
        source = edit['canonical_source']
        print('edits:', '; '.join(item['summary'] for item in edit['changes']['applied']))

        # 3. The extended model's card lists its dose target, covariate and every parameter.
        card = tool('get_model', {'source': source})
        targets = [d['target'] for d in card['interface']['dose_targets']]
        assert state in targets, targets
        assert {'wt'} <= {c['name'] for c in card['interface']['covariates']}
        cd19 = names['source_output_1']

        # 4. Typical subject: friendly doses in mg, a covariate and window readouts.
        induction = {'target': state, 'amount': q(1000, 'mg'), 'at': q(0, 'd'),
                     'every': q(14, 'd'), 'count': 2, 'duration': q(0.25, 'd')}
        readouts = [{'kind': 'max', 'output': cd19, 'from': q(28, 'd'), 'to': q(730, 'd')}]
        members = []
        for dose in (250, 500, 1000):
            for interval in (180, 270, 365):
                count = int(729.999 // interval)
                members.append({'name': f'{dose}mg_q{interval}d', 'doses': [induction, {
                    'target': state, 'amount': q(dose, 'mg'), 'at': q(interval, 'd'),
                    'every': q(interval, 'd'), 'count': count, 'duration': q(0.25, 'd')}]})
        grid = tool('compare_regimens', {'model': {'source': source}, 'end': q(730, 'd'),
                                         'covariates': {'wt': q(70, 'kg')},
                                         'regimens': members, 'readouts': readouts})
        peaks = {m['name']: m['result']['readouts']['results'][0]['value'] for m in grid['regimens']}
        meeting = sorted((n for n, v in peaks.items() if v <= TARGET),
                         key=lambda n: (int(n.split('mg')[0]) * int(729.999 // int(n.split('_q')[1][:-1])), n))
        print('window maximum of CD19+ (cells/mm3):', {k: round(v, 2) for k, v in peaks.items()})
        print('meets the illustrative target:', meeting)

        # 5. The smallest 270-day maintenance dose that meets the target, by bisection.
        found = tool('find_dose', {'model': {'source': source}, 'end': q(730, 'd'),
                                   'covariates': {'wt': q(70, 'kg')}, 'doses': [induction],
                                   'search': {'target': state, 'at': q(270, 'd'), 'every': q(270, 'd'),
                                              'count': 2, 'duration': q(0.25, 'd'),
                                              'lower': q(50, 'mg'), 'upper': q(2000, 'mg')},
                                   'constraint': {'readout': readouts[0], 'operator': 'at_most',
                                                  'threshold': TARGET}})
        print(f"find_dose q270 d: {found['status']}, {found['answer']['amount']['value']:.0f} mg "
              f"after {found['evaluations']} simulations")

        # 6. Sensitivity of the window maximum to the assumed tissue volume, with friendly fields.
        scan = tool('scan_parameters', {'model': {'source': source}, 'parameter': 'rtx_tissue_volume',
                                        'values': [q(1.5, 'L'), q(3, 'L'), q(6, 'L')],
                                        'end': q(730, 'd'), 'doses': members[4]['doses'],
                                        'covariates': {'wt': q(70, 'kg')}, 'readouts': readouts})
        print('500 mg q270 d, max CD19+ by tissue volume 1.5/3/6 L:',
              [round(p['readouts']['results'][0]['value'], 2) for p in scan['results']])
    finally:
        client.close()


if __name__ == '__main__':
    main()
