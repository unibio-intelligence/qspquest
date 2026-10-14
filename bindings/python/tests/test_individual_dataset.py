"""Explicit paired-row admission and independent likelihood/gradient checks."""
from copy import deepcopy
from hashlib import sha256
import importlib.util
import json
import math
from io import BytesIO
from pathlib import Path
import zipfile

import pytest
import qspquest as qsp
from qspquest.interchange import export_omex, import_omex, FIT_PROFILE, InterchangeError

ROOT = Path(__file__).resolve().parents[3]
MODEL = json.loads((ROOT/'conformance/models/paired-antibody-cell.json').read_bytes())
spec = importlib.util.spec_from_file_location('paired_example', ROOT/'docs/examples/paired_endpoint_fit.py')
example = importlib.util.module_from_spec(spec)
spec.loader.exec_module(example)


def dataset(solver='diffsol_bdf'):
    endpoints = {'pk': {'output': 'measured_concentration', 'value_unit': 'mg/L',
                       'error': {'additive_sd': qsp.quantity(.05, 'mg/L'), 'proportional_sd': 0}},
                 'pd': {'output': 'measured_cells', 'value_unit': '1/mm3',
                       'error': {'additive_sd': qsp.quantity(.2, '1/mm3'), 'proportional_sd': 0}}}
    records = [{'row': 2, 'endpoint': 'pd', 'value': 180},
               {'row': 0, 'endpoint': 'pk', 'value': 0},
               {'row': 2, 'endpoint': 'pk', 'value': 290},
               {'row': 0, 'endpoint': 'pd', 'value': 200}]
    return qsp.individual_fit_dataset(records, example.template(solver, example.STARTS[0]), endpoints=endpoints)


def test_unsorted_paired_records_defensive_copies_and_native_admission():
    data = dataset()
    assert [r['result_row'] for r in data.mapping] == [2, 0, 2, 0]
    assert [r['source_row'] for r in data.mapping] == [0, 1, 2, 3]
    assert data.mapping[1]['observation']['side'] == 'pre'
    request, mapping = data.request, data.mapping
    request['problem']['request']['objective']['observations'].clear()
    mapping[0]['source_row'] = 99
    assert len(data.request['problem']['request']['objective']['observations']) == 4
    assert data.mapping[0]['source_row'] == 0
    plan = data.plan(MODEL)
    assert plan['request']['problem']['kind'] == 'individual'
    assert plan['deferred_checks']


@pytest.mark.parametrize('invalid', [
    {'row': True, 'endpoint': 'pd', 'value': 2},
    {'row': 999, 'endpoint': 'pd', 'value': 2},
    {'row': 0, 'endpoint': 'unknown', 'value': 2},
    {'row': 0, 'endpoint': 'pd', 'value': float('nan')},
    {'row': 0, 'endpoint': 'pd', 'value': '2'},
    {'row': 0, 'endpoint': 'pd', 'value': 2, 'TIME': 1},
])
def test_invalid_source_records_are_not_repaired(invalid):
    error = {'output': 'measured_cells', 'value_unit': '1/mm3',
             'error': {'additive_sd': qsp.quantity(.2, '1/mm3'), 'proportional_sd': 0}}
    with pytest.raises(qsp.QSPQuestError):
        qsp.individual_fit_dataset([invalid], example.template('diffsol_bdf', example.STARTS[0]), endpoints={'pd': error})


@pytest.mark.parametrize('solver', ['diffsol_bdf', 'diffsol_rodas5p'])
def test_fit_profile_exact_roundtrip_two_solvers_and_plans(solver):
    request = dataset(solver).request
    raw = export_omex(MODEL, {}, fits={'paired': request})
    loaded = import_omex(raw, expected_sha256=sha256(raw).hexdigest())
    assert loaded.profile == FIT_PROFILE and loaded.scenarios == {}
    assert loaded.fits == {'paired': request}
    assert export_omex(loaded.imported_model, {}, fits=loaded.fits) == raw
    assert loaded.plan_fit('paired') == dataset(solver).plan(MODEL)
    with pytest.raises(InterchangeError):
        import_omex(raw, expected_sha256='0'*64)


@pytest.mark.parametrize('mutation', ['kind', 'unit', 'row', 'duplicate', 'unknown',
                                     'prior', 'lognormal', 'proportional', 'censoring'])
def test_statistical_profile_rejects_unmapped_semantics(mutation):
    request = dataset().request
    objective = request['problem']['request']['objective']
    observations = objective['observations']
    if mutation == 'kind': request['problem']['kind'] = 'pooled'
    if mutation == 'unit': observations[0]['value']['unit'] = 'mg'
    if mutation == 'row': observations[0]['row'] = 999
    if mutation == 'duplicate': observations.append(deepcopy(observations[0]))
    if mutation == 'unknown': request['problem']['request']['future_method'] = True
    if mutation == 'prior': objective['priors'] = [{}]
    if mutation == 'lognormal': observations[0]['error']['model'] = 'lognormal'
    if mutation == 'proportional': observations[0]['error']['proportional_sd'] = .1
    if mutation == 'censoring': observations[0]['censoring'] = 'left'
    with pytest.raises(InterchangeError):
        export_omex(MODEL, {}, fits={'invalid': request})


@pytest.mark.parametrize('mutation', ['unknown_profile', 'duplicate_json_key', 'unknown_request_field'])
def test_rehashed_archive_cannot_hide_unsupported_fit_contracts(mutation):
    raw = export_omex(MODEL, {}, fits={'paired': dataset().request})
    with zipfile.ZipFile(BytesIO(raw)) as archive:
        members = {name: archive.read(name) for name in archive.namelist()}
    index = json.loads(members['index.json'])
    path = index['fits'][0]['request']
    if mutation == 'unknown_profile':
        index['profile'] = 'qspquest.omex.individual-fit/v999'
    elif mutation == 'duplicate_json_key':
        members[path] = b'{"schema":"qspquest.fit/v0.1",'+members[path][1:]
    else:
        request = json.loads(members[path])
        request['problem']['request']['unknown'] = 'do not execute'
        members[path] = json.dumps(request).encode()
    index['members'][path] = sha256(members[path]).hexdigest()
    members['index.json'] = json.dumps(index).encode()
    changed = BytesIO()
    with zipfile.ZipFile(changed, 'w') as archive:
        for name, data in members.items():
            archive.writestr(name, data)
    with pytest.raises(InterchangeError) as error:
        import_omex(changed.getvalue())
    assert error.value.code == ('unsupported' if mutation == 'unknown_profile' else 'invalid_document')


@pytest.mark.parametrize('solver', ['diffsol_bdf', 'diffsol_rodas5p'])
def test_two_endpoint_objective_and_gradient_against_independent_exponentials(solver):
    source = '''model independent_two_outputs
time_unit = h
parameters
 k = 0.2 [1/h] estimated
states
 x = 10 [mg]
 y = 4 [mg/L]
dynamics
 d/dt(x) = -k*x
 d/dt(y) = -k*y
outputs
 mass = x [mg]
 concentration = y [mg/L]
end
'''
    run = qsp.run_request(end=2, times=[0, 1, 2], solver=solver, rtol=1e-11, atol=1e-13)
    observations, expected_nll, expected_gradient = [], 0., 0.
    for row, time in enumerate([0, 1, 2]):
        for name, amplitude, unit, sd in [('mass', 10, 'mg', 2), ('concentration', 4, 'mg/L', .3)]:
            value = amplitude*math.exp(-.4*time)
            predicted = amplitude*math.exp(-.2*time)
            difference = value-predicted
            expected_nll += .5*(difference/sd)**2+math.log(sd)+.5*math.log(2*math.pi)
            expected_gradient += -difference*(-time*predicted)/sd**2
            observations.append({'row': row, 'output': name, 'value': qsp.quantity(value, unit),
                'error': {'additive_sd': qsp.quantity(sd, unit), 'proportional_sd': 0}})
    objective = {'simulation': {'schema': 'qspquest.sensitivity/v0.1', 'run': run,
                 'with_respect_to': ['k'], 'derivative_absolute_tolerances': {
                     'k': {'x': qsp.quantity(1e-12, 'mg.h'),
                           'y': qsp.quantity(1e-12, 'mg.h/L')}}},
                 'observations': observations, 'priors': []}
    answer = qsp.CompiledSensitivities(source, ['k']).objective(objective)
    assert abs(answer['objective']-expected_nll) < 1e-7
    assert abs(answer['gradient'][0]['value']-expected_gradient) < 1e-6
    assert answer['observations'] == 6


def test_individual_template_requires_current_schemas_without_mutating_input():
    current = dataset().request
    before = deepcopy(current)
    assert export_omex(MODEL, {}, fits={'paired': current}) == export_omex(MODEL, {}, fits={'paired': before})
    assert current == before
    current['problem']['request']['objective']['observations'] = []
    endpoint = {'pd': {'output': 'measured_cells', 'value_unit': '1/mm3',
                      'error': {'additive_sd': qsp.quantity(.2, '1/mm3'), 'proportional_sd': 0}}}
    records = [{'row': 0, 'endpoint': 'pd', 'value': 200}]
    before = deepcopy(current)
    prepared = qsp.individual_fit_dataset(records, current, endpoints=endpoint).request
    assert current == before
    assert prepared['schema'] == 'qspquest.fit/v0.1'
    for level in range(3):
        simulation = current['problem']['request']['objective']['simulation']
        document = (current, simulation, simulation['run'])[level]
        tag = document['schema']
        for invalid in (tag.replace('qspquest.', 'other.'), tag.replace('v0.1', 'v99'), None, [tag], 1):
            document['schema'] = invalid
            rejected_input = deepcopy(current)
            with pytest.raises(qsp.QSPQuestError):
                qsp.individual_fit_dataset(records, current, endpoints=endpoint)
            assert current == rejected_input
        del document['schema']
        with pytest.raises(qsp.QSPQuestError):
            qsp.individual_fit_dataset(records, current, endpoints=endpoint)
        document['schema'] = tag


def test_individual_archive_rejects_unknown_nested_schemas():
    current = dataset().request
    simulation = current['problem']['request']['objective']['simulation']
    for document in (current, simulation, simulation['run']):
        tag = document['schema']
        for invalid in (tag.replace('qspquest.', 'other.'), tag.replace('v0.1', 'v99'), None):
            document['schema'] = invalid
            before = deepcopy(current)
            with pytest.raises((InterchangeError, qsp.QSPQuestError)):
                export_omex(MODEL, {}, fits={'paired': current})
            assert current == before
        document['schema'] = tag
