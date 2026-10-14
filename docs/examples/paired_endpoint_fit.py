"""Paired synthetic antibody/cell fitting with independent references and OMEX.

prepare fixes all data/requests before run; run retains every success/failure.
Requires qspquest[interchange]. Neither stage runs on import.
"""
import argparse
from copy import deepcopy
from hashlib import sha256
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODEL = ROOT / 'conformance/models/paired-antibody-cell.json'
GRID = [(0., 'pre'), (0., 'post')] + [(t, 'post') for t in
        [.25, 1., 2., 4., 8., 16., 32., 64., 128.]] + [
        (180., 'pre'), (180., 'post')] + [(t, 'post') for t in
        [181., 184., 196., 200., 240., 300., 360.]]
TRUTH = {'cl': .04, 'drug_effect': .003}
STARTS = [(.02, .0015), (.03, .0025), (.06, .005)]
UNITS = {'cl': '1/d', 'drug_effect': 'L/(mg.d)'}
BOUNDS = {'cl': (.01, .1), 'drug_effect': (.001, .006)}
SD = {'measured_concentration': .05, 'measured_cells': .2}
REFERENCE_STEPS = (512, 1024)


def encode(value):
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n').encode()


def write(path, value):
    with path.open('xb') as stream:
        stream.write(value if isinstance(value, bytes) else encode(value))


def quantity(value, unit):
    return {'value': value, 'unit': unit}


def reference(parameters, steps_per_day):
    """Independent piecewise two-equation RK4 in declared physical coordinates.

    Bolus jumps are applied between pre/post samples; cells stay continuous.
    Fixed illustrative values are declared here independently of engine AST.
    """
    c, cells, previous = 0., 200., 0.
    answers = {}
    doses = {0.: 1000. / 3., 180.: 500. / 3.}
    cl, effect = parameters['cl'], parameters['drug_effect']

    def rhs(a, b):
        return (-cl*a-.003*a*b, 200.*.0004-effect*a*b-.0004*b)

    for target in sorted({t for t, _ in GRID} | set(doses)):
        count = math.ceil((target-previous)*steps_per_day)
        if count:
            h = (target-previous)/count
            for _ in range(count):
                k1 = rhs(c, cells)
                k2 = rhs(c+h*k1[0]/2, cells+h*k1[1]/2)
                k3 = rhs(c+h*k2[0]/2, cells+h*k2[1]/2)
                k4 = rhs(c+h*k3[0], cells+h*k3[1])
                c += h*(k1[0]+2*k2[0]+2*k3[0]+k4[0])/6
                cells += h*(k1[1]+2*k2[1]+2*k3[1]+k4[1])/6
        answers[target, 'pre'] = (c, cells)
        c += doses.get(target, 0.)
        answers[target, 'post'] = (c, cells)
        previous = target
    result = [answers[item] for item in GRID]
    if any(not math.isfinite(v) or v < 0 for row in result for v in row):
        raise ValueError('Independent reference is nonfinite or negative')
    return result


def template(solver, start):
    import qspquest as qsp
    run = qsp.run_request(end=360, times=sorted({t for t, _ in GRID}),
        time_unit='d', solver=solver, parameters={n: quantity(v, UNITS[n])
            for n, v in zip(TRUTH, start)},
        administrations=[qsp.bolus('concentration', amount, 'mg', at=t, time_unit='d')
                         for t, amount in [(0, 1000), (180, 500)]], rtol=1e-10, atol=1e-12)
    run['regimen']['observations'] = [{'time': quantity(t, 'd'), 'side': side} for t, side in GRID]
    return {'schema': 'qspquest.fit/v0.1', 'problem': {'kind': 'individual', 'request': {
        'objective': {'simulation': {'schema': 'qspquest.sensitivity/v0.1', 'run': run,
            'with_respect_to': list(TRUTH), 'derivative_absolute_tolerances': {
                'cl': {'concentration': quantity(1e-10, 'mg.d/L'),
                       'cells': quantity(1e-10, 'd/mm3')},
                'drug_effect': {'concentration': quantity(1e-10, 'mg2.d/L2'),
                                'cells': quantity(1e-10, 'mg.d/(L.mm3)')}}},
            'observations': [], 'priors': []},
        'bounds': [{'parameter': n, 'lower': quantity(a, UNITS[n]), 'upper': quantity(b, UNITS[n])}
                   for n, (a, b) in BOUNDS.items()],
        'max_iterations': 200, 'max_evaluations': 2000, 'gradient_tolerance': 1e-6}}}


def reference_objective(objective):
    values = {n: objective['simulation']['run']['parameters'][n]['value'] for n in TRUTH}
    coarse, fine = [reference(values, steps) for steps in REFERENCE_STEPS]
    scale = max(abs(a-b)/(1e-7+1e-7*abs(b)) for x, y in zip(coarse, fine) for a, b in zip(x, y))
    if scale > 1:
        raise ValueError('Independent objective trajectory step-halving failed')
    nll = 0.
    for observation in objective['observations']:
        column = list(SD).index(observation['output'])
        prediction = fine[observation['row']][column]
        difference = observation['value']['value']-prediction
        sd = observation['error']['additive_sd']['value']
        nll += .5*(difference/sd)**2+math.log(sd)+.5*math.log(2*math.pi)
    return nll


def prepare(output):
    import qspquest as qsp
    from qspquest.interchange import export_omex, import_omex
    output.mkdir(parents=True, exist_ok=False)
    source = json.loads(MODEL.read_bytes())
    coarse, fine = [reference(TRUTH, steps) for steps in REFERENCE_STEPS]
    scaled = max(abs(a-b)/(1e-7+1e-7*abs(b)) for x, y in zip(coarse, fine) for a, b in zip(x, y))
    if scaled > 1:
        write(output/'reference-failure.json', {'maximum_scaled_difference': scaled})
        raise ValueError('Prespecified step-halving reference gate failed')
    endpoints = {n: {'output': n, 'value_unit': unit, 'error': {
        'additive_sd': quantity(SD[n], unit), 'proportional_sd': 0}}
        for n, unit in [('measured_concentration', 'mg/L'), ('measured_cells', '1/mm3')]}
    records = [{'row': row, 'endpoint': name, 'value': values[i]}
        for row, values in enumerate(fine) for i, name in enumerate(endpoints)]
    requests, mappings, plans = {}, {}, {}
    for solver in ['diffsol_bdf', 'diffsol_rodas5p']:
        for index, start in enumerate(STARTS):
            name = solver + '_start_' + str(index)
            dataset = qsp.individual_fit_dataset(records, template(solver, start), endpoints=endpoints)
            requests[name], mappings[name], plans[name] = dataset.request, dataset.mapping, dataset.plan(source)
    archive = export_omex(source, {}, fits=requests)
    digest = sha256(archive).hexdigest()
    imported = import_omex(archive, expected_sha256=digest)
    if imported.fits != requests or export_omex(imported.imported_model, {}, fits=imported.fits) != archive:
        raise ValueError('Exact paired-fit archive round trip failed')
    prepared = {'schema': 'qspquest.paired-fit-example/v1', 'model': source,
        'archive_sha256': digest, 'requests': requests, 'mappings': mappings,
        'plans': plans, 'truth': TRUTH, 'reference_steps_per_day': list(REFERENCE_STEPS),
        'reference_coarse': coarse, 'reference_fine': fine, 'reference_scaled_difference': scaled,
        'recovery_absolute_limits': {'cl': 1e-5, 'drug_effect': 1e-6},
        'qualification': 'synthetic paired-endpoint recovery only; clinical/statistical uncertainty not assessed'}
    raw = encode(prepared)
    write(output/'paired-fit.omex', archive)
    write(output/'prepared.json', raw)
    receipt = {'prepared_sha256': sha256(raw).hexdigest(), 'archive_sha256': digest,
               'registered_fits': len(requests), 'registered_observations_per_fit': len(records),
               'fit_executed': False, 'reference_gate_passed': True}
    write(output/'preparation.json', receipt)
    return receipt


def run(prepared_path, expected_sha256, output):
    import qspquest as qsp
    from qspquest import _native
    from qspquest.interchange import import_omex, export_omex, InterchangeError
    raw = prepared_path.read_bytes()
    if sha256(raw).hexdigest() != expected_sha256:
        raise ValueError('Prepared bytes differ from the trusted preparation digest')
    prepared = json.loads(raw)
    if prepared['schema'] != 'qspquest.paired-fit-example/v1':
        raise ValueError('Unsupported preparation profile')
    output.mkdir(parents=True, exist_ok=False)
    native_path = Path(_native.__file__)
    native_sha = sha256(native_path.read_bytes()).hexdigest()
    archive = import_omex(prepared_path.with_name('paired-fit.omex'), expected_sha256=prepared['archive_sha256'])
    if archive.fits != prepared['requests'] or len(archive.fits) != 6:
        raise ValueError('Registered fit inventory changed')
    cases = []
    for name, request in sorted(archive.fits.items()):
        try:
            if archive.plan_fit(name) != prepared['plans'][name]:
                raise ValueError('Native admission identity changed since preparation; prepare again with the selected artifact')
            result = archive.fit(name)
            write(output/(name+'-result.json'), result)
            fit = result['fit']
            identity = fit['objective']['identity']
            if identity['model_content_hash'] != qsp.CompiledModel(prepared['model']).model_content_hash:
                raise ValueError('Native result model identity differs')
            recovered = all(fit['parameters'][p]['unit'] == UNITS[p] and abs(fit['parameters'][p]['value']-truth) <= prepared['recovery_absolute_limits'][p]
                            for p, truth in prepared['truth'].items())
            simulation = deepcopy(request['problem']['request']['objective']['simulation'])
            simulation['run']['parameters'].update(fit['parameters'])
            predicted = qsp.CompiledSensitivities(prepared['model'], list(TRUTH)).run(simulation)['result']
            if predicted['times'] != [t for t, _ in GRID] or predicted['sides'] != [s for _, s in GRID]:
                raise ValueError('Native prediction grid differs')
            series = {s['name']: s for s in predicted['outputs']}
            write(output/(name+'-predictions.json'), predicted)
            reference_values = {p: fit['parameters'][p]['value'] for p in TRUTH}
            independent = reference(reference_values, REFERENCE_STEPS[1])
            trajectory_error = max(abs(series[n]['values'][row]-values[column]) /
                (1e-6+1e-5*abs(values[column])) for row, values in enumerate(independent)
                for column, n in enumerate(SD))
            # Compare a non-optimal starting point, where the likelihood gradient
            # is informative. All finite-difference steps/limits are prespecified.
            objective = deepcopy(request['problem']['request']['objective'])
            native_objective = qsp.CompiledSensitivities(prepared['model'], list(TRUTH)).objective(objective)
            independent_nll = reference_objective(objective)
            objective_error = abs(native_objective['objective']-independent_nll)/(1e-4+1e-6*abs(independent_nll))
            gradients = []
            native_gradient = {g['parameter']: g['value'] for g in native_objective['gradient']}
            for parameter in TRUTH:
                start = objective['simulation']['run']['parameters'][parameter]['value']
                step = start*1e-4
                plus, minus = deepcopy(objective), deepcopy(objective)
                plus['simulation']['run']['parameters'][parameter]['value'] = start+step
                minus['simulation']['run']['parameters'][parameter]['value'] = start-step
                derivative = (reference_objective(plus)-reference_objective(minus))/(2*step)
                gradient_error = abs(native_gradient[parameter]-derivative)/(1e-3+1e-4*abs(derivative))
                gradients.append({'parameter': parameter, 'native': native_gradient[parameter],
                    'independent': derivative, 'scaled_error': gradient_error, 'step': step})
            residuals, endpoint_nll = [], {n: 0. for n in SD}
            for index, obs in enumerate(request['problem']['request']['objective']['observations']):
                source = prepared['mappings'][name][index]
                if source['source_row'] != index or source['result_row'] != obs['row'] or source['output'] != obs['output']:
                    raise ValueError('Source row mapping differs')
                endpoint, row = obs['output'], obs['row']
                if series[endpoint]['unit'] != obs['value']['unit']:
                    raise ValueError('Endpoint prediction unit differs')
                value = series[endpoint]['values'][row]
                difference = obs['value']['value']-value
                sd = obs['error']['additive_sd']['value']
                term = .5*(difference/sd)**2+math.log(sd)+.5*math.log(2*math.pi)
                endpoint_nll[endpoint] += term
                residuals.append({**source, 'unit': obs['value']['unit'], 'observed': obs['value']['value'],
                    'predicted': value, 'residual': difference, 'standardized_residual': difference/sd,
                    'negative_log_likelihood': term})
            objective_agrees = abs(sum(endpoint_nll.values())-fit['objective']['objective']) <= 1e-6
            independent_agrees = trajectory_error <= 1 and objective_error <= 1 and all(g['scaled_error'] <= 1 for g in gradients)
            passed = fit['status'] == 'converged' and recovered and objective_agrees and independent_agrees
            diagnostic = {'status': fit['status'], 'parameter_recovery': recovered, 'likelihood_sum_agrees': objective_agrees,
                'endpoint_negative_log_likelihood': endpoint_nll, 'observations': residuals,
                'independent_trajectory_scaled_error': trajectory_error,
                'independent_objective_scaled_error': objective_error, 'independent_gradients': gradients,
                'interpretation': 'Uncensored fixed-error individual residuals; not CWRES/NPDE, uncertainty or clinical qualification.'}
            write(output/(name+'-diagnostics.json'), diagnostic)
            cases.append({'case': name, 'passed': passed, 'identity': identity})
        except Exception as error:
            failure = {'case': name, 'passed': False, 'error_type': type(error).__name__, 'message': str(error)}
            write(output/(name+'-failure.json'), failure)
            cases.append(failure)
    controls = []
    base = deepcopy(next(iter(archive.fits.values())))
    for name in ['wrong_unit', 'wrong_row', 'duplicate_row', 'unsupported_method']:
        invalid = deepcopy(base)
        obs = invalid['problem']['request']['objective']['observations']
        if name == 'wrong_unit': obs[0]['value']['unit'] = 'mg'
        if name == 'wrong_row': obs[0]['row'] = len(GRID)
        if name == 'duplicate_row': obs.append(deepcopy(obs[0]))
        if name == 'unsupported_method': invalid['problem']['kind'] = 'saem'
        try:
            export_omex(prepared['model'], {}, fits={'invalid': invalid})
            controls.append({'case': name, 'passed': False, 'message': 'invalid fit was admitted'})
        except InterchangeError as error:
            controls.append({'case': name, 'passed': error.code == ('unsupported' if name == 'unsupported_method' else 'invalid_document'),
                             'code': error.code, 'message': str(error)})
    limited = deepcopy(base)
    limited['problem']['request'].update(max_evaluations=1, max_iterations=1)
    try:
        names = limited['problem']['request']['objective']['simulation']['with_respect_to']
        result = qsp.CompiledSensitivities(prepared['model'], names).fit(limited)
        write(output/'nonconvergence-result.json', result)
        controls.append({'case': 'nonconvergence', 'passed': result['fit']['status'] != 'converged'})
    except qsp.QSPQuestError as error:
        controls.append({'case': 'nonconvergence', 'passed': error.code == 'budget', 'diagnostic': error.diagnostic})
    unchanged = sha256(native_path.read_bytes()).hexdigest() == native_sha
    report = {'passed': all(c['passed'] for c in cases+controls) and unchanged,
        'registered_fit_denominator': 6, 'registered_control_denominator': 5,
        'cases': cases, 'controls': controls, 'prepared_sha256': expected_sha256,
        'native_extension_sha256': native_sha, 'native_extension_unchanged': unchanged,
        'scientific_scope': prepared['qualification']}
    write(output/'report.json', report)
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    p = commands.add_parser('prepare')
    p.add_argument('--output', type=Path, required=True)
    r = commands.add_parser('run')
    r.add_argument('--prepared', type=Path, required=True)
    r.add_argument('--prepared-sha256', required=True)
    r.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        report = prepare(args.output) if args.command == 'prepare' else run(args.prepared, args.prepared_sha256, args.output)
        print(json.dumps(report, indent=2))
        if report.get('passed') is False:
            raise SystemExit(1)
    except Exception as error:
        parser.exit(1, f'{type(error).__name__}: {error}\n')
