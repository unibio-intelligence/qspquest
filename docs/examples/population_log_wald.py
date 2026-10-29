"""Fit a synthetic population and inspect positive fixed-effect log-Wald intervals."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from population_two_effects import prepare, NAMES


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, help='Use this native CLI instead of the installed Python binding.')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='qspquest-log-wald-') as temporary:
        directory = Path(temporary) / 'population'
        prepare(directory)
        path = directory / 'saem-fixed-request.json'
        request = json.loads(path.read_text())
        request['problem']['request']['uncertainty'] = True
        path.write_text(json.dumps(request, allow_nan=False))
        source = directory / 'model.qsp'
        if args.cli:
            completed = subprocess.run([str(args.cli.resolve()), 'fit', str(source), str(path)],
                                       check=True, capture_output=True, text=True)
            result = json.loads(completed.stdout)
        else:
            from qspquest import CompiledSensitivities
            result = CompiledSensitivities(source.read_text(), NAMES).fit(request)
        population = result['population']
        assert population['status'] == 'converged'
        intervals = population['fixed_effect_intervals']
        assert intervals['method'] == 'positive_fixed_effect_log_wald_delta_v1'
        assert intervals['confidence_level'] == 0.95
        assert [row['name'] for row in intervals['intervals']] == NAMES
        for row in intervals['intervals']:
            assert 0 < row['lower'] < row['estimate'] < row['upper']
            assert row['unit'] == '1'
        print(json.dumps(intervals, indent=2, allow_nan=False))
        print('Synthetic demonstration; interval coverage in another design is unproved.')


if __name__ == '__main__':
    main()
