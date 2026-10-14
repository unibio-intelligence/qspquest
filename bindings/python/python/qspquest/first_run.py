"""Run the wheel's complete synthetic example and retain its replay bundle."""
import argparse
import json
from pathlib import Path

from .result_bundle import write_bundle

SOURCE = 'model synthetic_one_compartment\ntime_unit = h\ndescription = "Synthetic linear PK fixture; no drug-specific parameters."\n\nparameters\n  cl = 0.3 [L/h] log estimated\n  v = 3 [L] log estimated\n\nstates\n  central = 0 [mg]\n\ndynamics\n  d/dt(central) = (input_central - ((cl / v) * central))\n\noutputs\n  cp = (central / v) [mg/L]\n\ndosing\n  central [mg] = 1\nend\n'
REQUEST = {'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf', 'budgets': {'solver_callbacks': 1000000, 'events': 100, 'output_values': 10000}, 'regimen': {'end': {'value': 24, 'unit': 'h'}, 'samples': [], 'observations': [{'time': {'value': 0, 'unit': 'h'}, 'side': 'post'}, {'time': {'value': 2, 'unit': 'h'}, 'side': 'post'}, {'time': {'value': 8, 'unit': 'h'}, 'side': 'post'}, {'time': {'value': 24, 'unit': 'h'}, 'side': 'post'}], 'administrations': [{'target': 'central', 'time': {'value': 0, 'unit': 'h'}, 'amount': {'value': 6, 'unit': 'mg'}, 'delivery': {'kind': 'bolus'}, 'bioavailability': 1.0}], 'rtol': 1e-10, 'atol': 1e-12}}


def synthetic_inputs():
    """Return the synthetic source and an independent unit-bearing request."""
    return SOURCE, json.loads(json.dumps(REQUEST))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=Path('developer-results'))
    args = parser.parse_args()
    if args.out.exists() or args.out.is_symlink():
        parser.error('Choose a new output directory: ' + str(args.out))
    from . import CompiledModel
    source, request = synthetic_inputs()
    result = CompiledModel(source).run(request)
    report = write_bundle(args.out, result, source, request, producer='python')
    print('time_h,side,cp_mg_per_L')
    values = next(column['values'] for column in result['outputs'] if column['name'] == 'cp')
    for hour, side, value in zip(result['times'], result['sides'], values):
        print(f'{hour:g},{side},{value:.6f}')
    print('Analytic check passed. Saved six files in ' + report['directory'])


if __name__ == '__main__':
    main()
