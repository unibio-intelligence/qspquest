"""Synthetic first-run checks refuse incomplete or changed numerical evidence."""
import copy
import importlib.util
import json
import math
from pathlib import Path

import pytest
from qspquest.first_run import synthetic_inputs
from qspquest.result_bundle import check_result, write_bundle


def result():
    return {'schema': 'qspquest.result/v0.1', 'time_unit': 'h', 'times': [0, 2, 8, 24],
            'sides': ['post'] * 4, 'identity': {'fixture': 'synthetic reader test'},
            'outputs': [{'name': 'cp', 'unit': 'mg/L', 'values': [2 * math.exp(-0.1 * t) for t in [0, 2, 8, 24]]}]}


@pytest.mark.parametrize('mutation', ['truncated', 'wrong_unit', 'boolean', 'nonfinite', 'event_side', 'numerical', 'missing_identity'])
def test_refuses_incomplete_or_wrong_result(mutation):
    value = copy.deepcopy(result())
    if mutation == 'truncated': value['outputs'][0]['values'].pop()
    if mutation == 'wrong_unit': value['outputs'][0]['unit'] = 'ug/L'
    if mutation == 'boolean': value['outputs'][0]['values'][0] = True
    if mutation == 'nonfinite': value['outputs'][0]['values'][0] = float('nan')
    if mutation == 'event_side': value['sides'][0] = 'pre'
    if mutation == 'numerical': value['outputs'][0]['values'][0] += 1e-5
    if mutation == 'missing_identity': value['identity'] = {}
    with pytest.raises(ValueError): check_result(value)


@pytest.mark.parametrize('producer', ['cli', 'python', 'mcp'])
def test_roundtrip_and_refuses_overwrite_or_changed_csv(tmp_path, producer):
    source, request = synthetic_inputs()
    directory = tmp_path / producer
    report = write_bundle(directory, result(), source, request, producer=producer)
    assert len(report['files']) == 6
    spec = importlib.util.spec_from_file_location('table_reader', Path(__file__).resolve().parents[3] / 'docs/examples/table_bundle.py')
    reader = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(reader)
    parsed = reader.read_table(directory / 'first-run.csv')
    assert parsed['rows'][0] == {'time_h': 0.0, 'side': 'post', 'cp_mg_per_L': 2.0}
    assert parsed['manifest']['qualification']['qualified'] is False
    assert json.loads((directory / 'request.json').read_bytes()) == request
    with pytest.raises(FileExistsError): write_bundle(directory, result(), source, request, producer=producer)
    with (directory / 'first-run.csv').open('ab') as stream: stream.write(b'\n')
    with pytest.raises(ValueError, match='byte count or SHA-256'): reader.read_table(directory / 'first-run.csv')
