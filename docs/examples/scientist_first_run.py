"""Run a packaged CLI and save the complete synthetic first-run table and provenance."""
import argparse
import json
import os
from pathlib import Path
import subprocess

try:
    from result_bundle import write_bundle  # Included beside this script in the CLI download.
except ModuleNotFoundError:
    from qspquest.result_bundle import write_bundle


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    here = Path(__file__).resolve().parent
    parser.add_argument('--binary', type=Path, default=here / ('qspquest.exe' if os.name == 'nt' else 'qspquest'))
    parser.add_argument('--model', type=Path, default=here / 'synthetic-one-compartment.qsp')
    parser.add_argument('--request', type=Path, default=here / 'first-run.json')
    parser.add_argument('--out', type=Path, default=Path('scientist-results'))
    args = parser.parse_args()
    for path in (args.binary, args.model, args.request):
        if not path.is_file(): parser.error('Use the complete extracted CLI download; missing file: ' + str(path))
    if args.out.exists() or args.out.is_symlink(): parser.error('Choose a new output directory: ' + str(args.out))
    source = args.model.read_text(encoding='utf-8')
    request_bytes = args.request.read_bytes()
    request = json.loads(request_bytes)
    env = {key: value for key, value in os.environ.items() if not key.startswith('QSPQUEST_')}
    process = subprocess.run([str(args.binary.resolve()), 'run', str(args.model.resolve()), str(args.request.resolve())],
        env=env, capture_output=True, timeout=120)
    if process.returncode:
        raise SystemExit('CLI first run failed: ' + process.stderr.decode('utf-8', errors='replace'))
    result = json.loads(process.stdout)
    if args.model.read_text(encoding='utf-8') != source or args.request.read_bytes() != request_bytes:
        raise SystemExit('The input files changed during execution; restore them and repeat in a new directory')
    report = write_bundle(args.out, result, source, request, producer='cli')
    print('time_h,side,cp_mg_per_L')
    values = next(column['values'] for column in result['outputs'] if column['name'] == 'cp')
    for hour, side, value in zip(result['times'], result['sides'], values):
        print(f'{hour:g},{side},{value:.6f}')
    print('Analytic check passed. Saved six files in ' + report['directory'])


if __name__ == '__main__':
    main()
