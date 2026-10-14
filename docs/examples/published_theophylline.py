"""Fit one published theophylline subject from the pinned validation companion."""
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import subprocess
import zipfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--validation', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    engine = Path(__file__).resolve().parents[2]
    pin = json.loads((engine / '.github/suite-compatibility.json').read_text())['components']['validation']
    manifest_raw = (args.validation / 'manifest.json').read_bytes()
    if digest(manifest_raw) != pin['manifest_sha256']:
        raise ValueError('Validation snapshot differs from the engine suite pin; select the pinned revision')
    manifest = json.loads(manifest_raw)
    archive_name = 'inputs/individual-fitting.omex'
    raw = (args.validation / archive_name).read_bytes()
    if digest(raw) != manifest['files'][archive_name]:
        raise ValueError('Validation archive checksum differs')
    prefix = 'fitting-examples-20261005/theo_sd/1/'
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        checks = json.loads(archive.read('checksums.json'))
        inputs = {}
        for name in ['model.qsp', 'diffsol_bdf-request.json', 'data.csv']:
            payload = archive.read(prefix + name)
            if digest(payload) != checks[prefix + name]:
                raise ValueError('Input checksum differs: ' + name)
            inputs[name] = payload
    args.out.mkdir(parents=False, exist_ok=False)
    for name, payload in inputs.items():
        (args.out / name).write_bytes(payload)
    command = [str(args.cli.resolve()), 'fit', str(args.out / 'model.qsp'),
               str(args.out / 'diffsol_bdf-request.json')]
    process = subprocess.run(command, capture_output=True, text=True, check=True)
    result = json.loads(process.stdout)
    (args.out / 'fit.json').write_text(json.dumps(result, indent=2) + '\n')
    if result['fit']['status'] != 'converged':
        raise ValueError('Fit did not converge; inspect fit.json')
    request = json.loads(inputs['diffsol_bdf-request.json'])
    run = request['problem']['request']['objective']['simulation']['run']
    run['parameters'].update(result['fit']['parameters'])
    prediction_request = args.out / 'prediction-request.json'
    prediction_request.write_text(json.dumps(run, indent=2) + '\n')
    process = subprocess.run([str(args.cli.resolve()), 'run', str(args.out / 'model.qsp'),
                              str(prediction_request)], capture_output=True, text=True, check=True)
    prediction = json.loads(process.stdout)
    (args.out / 'prediction.json').write_text(json.dumps(prediction, indent=2) + '\n')
    observations = [row for row in csv.DictReader(io.StringIO(inputs['data.csv'].decode()))
                    if row['EVID'] == '0']
    values = next(x['values'] for x in prediction['outputs'] if x['name'] == 'cp')
    times = prediction['times']
    if len(values) != len(observations) or times != [float(r['TIME']) for r in observations]:
        raise ValueError('Prediction and observation coordinates differ')
    with (args.out / 'observed-predicted.csv').open('w', newline='') as stream:
        writer = csv.writer(stream);writer.writerow(['time_h', 'observed', 'predicted', 'residual'])
        for row, value in zip(observations, values):
            observed = float(row['DV']);writer.writerow([row['TIME'], observed, value, observed - value])
    receipt = {'dataset': 'theo_sd', 'subject': 1, 'observations': len(observations),
               'validation_commit': pin['commit'], 'validation_manifest_sha256': pin['manifest_sha256'],
               'inputs': {name: digest(value) for name, value in inputs.items()},
               'fit': result['fit'], 'prediction_identity': prediction['identity'],
               'units': 'Source numerical coordinates retained; see the tutorial before assigning physical units.'}
    (args.out / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps({'status': 'converged', 'observations': len(observations),
                      'parameters': result['fit']['parameters'], 'output': str(args.out)}))


if __name__ == '__main__':
    main()
