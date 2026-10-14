"""Native normalization identities without fitting or numerical integration."""
from copy import deepcopy
import json
from pathlib import Path
import pytest
import qspquest as qsp

MODEL = json.loads((Path(__file__).resolve().parents[3] / 'examples/synthetic/absorption.model.json').read_bytes())


def scalar_request():
    parameter = next(p for p in MODEL['parameters'] if not p.get('fixed', False))
    value = parameter['default']['value']
    unit = parameter['default']['unit']
    return {'schema': 'qspquest.scalar-fit/v0.1', 'run': qsp.run_request(end=1, times=[1]),
            'parameter': parameter['name'], 'lower': qsp.quantity(value/2, unit), 'upper': qsp.quantity(value*2, unit),
            'observations': [{'row': 0, 'output': MODEL['outputs'][0]['name'],
                              'value': qsp.quantity(1, MODEL['outputs'][0]['unit']),
                              'error': {'additive_sd': qsp.quantity(.1, MODEL['outputs'][0]['unit']), 'proportional_sd': 0}}],
            'max_iterations': 200, 'absolute_tolerance': 1e-8}


def test_native_identity_normalizes_defaults_and_binds_all_values():
    request = scalar_request()
    identity = qsp.fitting_identity(MODEL, request, 'fit_scalar')
    expanded = deepcopy(request)
    expanded['run']['request_id'] = None
    expanded['run']['seed'] = None
    expanded['run']['accuracy_control'] = 'local'
    expanded['observations'][0]['error']['model'] = 'normal'
    expanded['observations'][0]['censoring'] = None
    assert identity == qsp.fitting_identity(MODEL, expanded, 'fit_scalar')
    assert identity['model_content_hash'] == qsp.CompiledModel(MODEL).model_content_hash
    changed = deepcopy(request)
    changed['observations'][0]['value']['value'] += .1
    assert identity['native_request_sha256'] != qsp.fitting_identity(MODEL, changed, 'fit_scalar')['native_request_sha256']


def test_identity_rejects_unknown_fields_operation_and_population_kind():
    request = scalar_request()
    request['unknown'] = True
    with pytest.raises(qsp.QSPQuestError):
        qsp.fitting_identity(MODEL, request, 'fit_scalar')
    with pytest.raises(qsp.QSPQuestError):
        qsp.fitting_identity(MODEL, scalar_request(), 'fit_population')
    with pytest.raises(qsp.QSPQuestError):
        qsp.fitting_identity(MODEL, {'schema': 'qspquest.fit/v0.1', 'problem': {'kind': 'population', 'request': {}}}, 'fit_individual')
    with pytest.raises(qsp.QSPQuestError):
        qsp.fitting_identity(MODEL, scalar_request(), 'fit_scalar', result={'status': 'converged'})
