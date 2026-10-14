"""Prepare and summarize a synthetic native two-effect fitting walkthrough."""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import random


MODEL = '''model synthetic_random_intercept_slope
time_unit = h
parameters
 theta_intercept = 4.0551999668446745 [1] estimated
 theta_slope = 2.0137527074704766 [1] estimated
 center = 1 [h] fixed
 scale = 1 [h] fixed
 decay = 0 [1/h] fixed
states
 anchor = 0 [1]
dynamics
 d/dt(anchor) = -decay * anchor
outputs
 z = log(theta_intercept) + ((time - center) / scale) * log(theta_slope) [1]
end
'''
NAMES = ['theta_intercept', 'theta_slope']
ROUTES = ['focei', 'saem-fixed', 'saem-fitted']


def quantity(value, unit='1'):
    return {'value': value, 'unit': unit}


def save(path, value):
    with path.open('x') as stream:
        stream.write(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(directory):
    directory.mkdir(exist_ok=False)
    model = directory / 'model.qsp'
    model.write_text(MODEL)
    rng = random.Random(314159)
    omega = [[0.16, 0.35 * math.sqrt(0.16 * 0.09)],
             [0.35 * math.sqrt(0.16 * 0.09), 0.09]]
    n, m = 32, 6
    times = [2 * j / (m - 1) for j in range(m)]
    rows = []
    for _ in range(n):
        a, b = rng.gauss(0, 1), rng.gauss(0, 1)
        eta_intercept = math.sqrt(omega[0][0]) * a
        eta_slope = omega[1][0] / math.sqrt(omega[0][0]) * a
        eta_slope += math.sqrt(omega[1][1] - omega[1][0] ** 2 / omega[0][0]) * b
        rows.append([1.4 + eta_intercept + (t - 1) * (0.7 + eta_slope)
                     + rng.gauss(0, 0.25) for t in times])
    with (directory / 'observations.csv').open('x', newline='') as stream:
        writer = csv.writer(stream)
        writer.writerow(['subject_index', 'row', 'time_h', 'z_dimensionless'])
        writer.writerows([i, j, times[j], value]
                         for i, row in enumerate(rows) for j, value in enumerate(row))
    for route in ROUTES:
        method = 'focei' if route == 'focei' else 'saem'
        fitted_sd = route == 'saem-fitted'
        sd = 0.2 if fitted_sd else 0.25
        simulation = {'schema': 'qspquest.sensitivity/v0.1', 'with_respect_to': NAMES,
            'derivative_absolute_tolerances': {name: {'anchor': quantity(1e-12)} for name in NAMES},
            'run': {'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf',
                'parameters': {name: quantity(math.exp(mu - 0.4)) for name, mu in zip(NAMES, [1.4, 0.7])},
                'budgets': {'solver_callbacks': 10000, 'events': 100, 'output_values': 1000},
                'regimen': {'start': quantity(0, 'h'), 'end': quantity(2, 'h'),
                    'rtol': 1e-10, 'atol': 1e-12, 'samples': [], 'administrations': [],
                    'observations': [{'time': quantity(t, 'h'), 'side': 'post'} for t in times]}}}
        body = {'subjects': [{'simulation': simulation, 'priors': [], 'observations': [
            {'row': j, 'output': 'z', 'value': quantity(value), 'error': {
                'model': 'normal', 'additive_sd': quantity(sd), 'proportional_sd': 0}}
            for j, value in enumerate(row)]} for row in rows],
            'fixed_effects': [{'parameter': name, 'lower': quantity(math.exp(low)), 'upper': quantity(math.exp(high))}
                             for name, low, high in zip(NAMES, [-0.2, -0.7], [3.0, 2.1])],
            'random_effects': [{'parameter': name} for name in NAMES],
            'omega': omega if method == 'focei' else [[0.5 * value for value in row] for row in omega],
            'omega_diagonal_bounds': [[0.16, 0.16], [0.09, 0.09]] if method == 'focei'
                                      else [[0.001, 0.8], [0.001, 0.8]],
            'eta_bound': 5, 'max_evaluations': 100000 if method == 'focei' else 200000,
            'max_iterations': 300 if method == 'focei' else 1200,
            'gradient_tolerance': 0.0005 if method == 'focei' else 0.003,
            'seed': 40117, 'uncertainty': False}
        if method == 'saem':
            body['burn_in_iterations'] = 200
        if fitted_sd:
            body['additive_error_fit'] = {'output': 'z', 'initial': quantity(sd),
                                        'lower': quantity(0.05), 'upper': quantity(0.8)}
        save(directory / (route + '-request.json'),
             {'schema': 'qspquest.fit/v0.1', 'problem': {'kind': method, 'request': body}})
    save(directory / 'design.json', {'synthetic': True, 'data_seed': 314159,
        'subjects': n, 'observations_per_subject': m, 'output': {'name': 'z', 'unit': '1'},
        'truth': {'theta_intercept': math.exp(1.4), 'theta_slope': math.exp(0.7),
                  'omega': omega, 'additive_sd': 0.25},
        'model_sha256': digest(model), 'observations_sha256': digest(directory / 'observations.csv'),
        'statistical_qualification': False})
    print(f'Prepared {n} synthetic subjects with {m} rows each in {directory}.')


def summarize(directory):
    if (directory / 'summary.json').exists():
        raise ValueError('summary.json already exists. Use a new demonstration directory to repeat the workflow.')
    design = json.loads((directory / 'design.json').read_text())
    if digest(directory / 'model.qsp') != design['model_sha256'] or digest(directory / 'observations.csv') != design['observations_sha256']:
        raise ValueError('Model or observation file changed after preparation. Prepare a new demonstration directory.')
    summaries = []
    for route in ROUTES:
        path = directory / (route + '-result.json')
        payload = json.loads(path.read_text())
        pop = payload['population']
        if payload['schema'] != 'qspquest.fit-result/v0.1' or pop['status'] != 'converged':
            raise ValueError(f'{route}: status is {pop.get("status")}. Keep the raw result for diagnosis; do not interpret it as a final fit.')
        expected_kind = ('focei_objective_function_value' if route == 'focei'
                         else 'saem_marginal_objective_function_value')
        if pop['objective_kind'] != expected_kind:
            raise ValueError(f'{route}: objective is {pop["objective_kind"]}; this walkthrough requires a marginal objective.')
        theta = [pop['fixed_effects'][name]['value'] for name in NAMES]
        omega = pop['omega']
        sd = pop.get('estimated_residual_error', {}).get('additive_sd', {'value': 0.25})['value']
        rho = omega[0][1] / math.sqrt(omega[0][0] * omega[1][1])
        if not all(math.isfinite(v) for v in [*theta, *[v for row in omega for v in row], sd, rho]) or min(theta + [omega[0][0], omega[1][1], sd]) <= 0 or abs(rho) >= 1:
            raise ValueError(f'{route}: invalid estimates or covariance. Inspect the raw result.')
        summaries.append({'route': route, 'status': pop['status'],
            'theta_intercept': theta[0], 'theta_slope': theta[1], 'omega': omega, 'correlation': rho,
            'additive_sd': sd, 'objective_kind': pop['objective_kind'],
            'fit_request_hash': pop['fit_request_hash'], 'request_sha256': digest(directory / (route + '-request.json')),
            'result_sha256': digest(path), 'saem_diagnostics': pop.get('saem_diagnostics'),
            'statistical_qualification': False})
    save(directory / 'summary.json', {'synthetic': True, 'design_sha256': digest(directory / 'design.json'),
        'model_sha256': design['model_sha256'], 'observations_sha256': design['observations_sha256'],
        'results': summaries, 'interpretation': 'Synthetic workflow demonstration. Convergence does not establish statistical calibration or suitability for scientific inference.'})
    print('route,theta_intercept,theta_slope,omega_intercept,omega_slope,correlation,additive_sd')
    for value in summaries:
        print(f"{value['route']},{value['theta_intercept']:.4f},{value['theta_slope']:.4f},"
              f"{value['omega'][0][0]:.4f},{value['omega'][1][1]:.4f},"
              f"{value['correlation']:.4f},{value['additive_sd']:.4f}")
    print('Saved summary.json with file identities and native request hashes. This is a synthetic demonstration.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['prepare', 'summarize'])
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    try:
        (prepare if args.command == 'prepare' else summarize)(args.directory)
    except (OSError, ValueError, KeyError, TypeError, IndexError, ZeroDivisionError) as error:
        parser.exit(2, f'Population walkthrough: {error}\n')
