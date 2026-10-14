"""Save the bounded synthetic first-run result with replay inputs and table metadata.

This module uses only the standard library and is also included in native
first-run downloads. Checksums identify bytes; they do not authenticate or
qualify a producer. This writer is limited to the stated synthetic trajectory.
"""
import csv
import hashlib
import io
import json
import math
from pathlib import Path

MODEL_ID = 'synthetic_one_compartment'
ABSOLUTE_LIMIT = 1e-8


def encode(value):
    return (json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + '\n').encode('utf-8')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def check_result(result):
    """Check exactly four post-dose concentrations against 2 exp(-0.1 t)."""
    if not isinstance(result, dict):
        raise ValueError('The first run requires a complete result object')
    if (result.get('schema') != 'qspquest.result/v0.1' or result.get('time_unit') != 'h'
            or result.get('times') != [0, 2, 8, 24] or result.get('sides') != ['post'] * 4
            or any(type(hour) not in (int, float) for hour in result.get('times', []))
            or not isinstance(result.get('identity'), dict) or not result['identity']):
        raise ValueError('The first run requires its complete four-row synthetic result, units, event sides and identity')
    outputs = result.get('outputs')
    if not isinstance(outputs, list) or any(not isinstance(column, dict) for column in outputs):
        raise ValueError('The first run requires native output columns')
    columns = [column for column in outputs if column.get('name') == 'cp']
    if (len(columns) != 1 or columns[0].get('unit') != 'mg/L'
            or not isinstance(columns[0].get('values'), list) or len(columns[0]['values']) != 4):
        raise ValueError('The first run requires four cp concentrations in mg/L')
    values = columns[0]['values']
    if any(type(value) not in (float, int) or not math.isfinite(value) for value in values):
        raise ValueError('Synthetic concentrations must be finite numbers')
    errors = [abs(value - 2 * math.exp(-0.1 * hour)) for hour, value in zip(result['times'], values)]
    if max(errors) > ABSOLUTE_LIMIT:
        raise ValueError('Synthetic concentration exceeds the 1e-8 mg/L analytical limit')
    return {'status': 'passed', 'model_id': MODEL_ID, 'observations': 4,
            'analytic_absolute_limit_mg_L': ABSOLUTE_LIMIT, 'max_absolute_error_mg_L': max(errors),
            'scope': 'This synthetic four-observation numerical case only; no drug-specific, fitting or general scientific qualification'}


def write_bundle(directory, result, source, request, *, producer):
    """Write a new directory; existing paths are refused rather than replaced."""
    if producer not in ('cli', 'python', 'mcp'):
        raise ValueError('First-run producer must be cli, python or mcp')
    checks = check_result(result)
    if not isinstance(source, str) or not isinstance(request, dict):
        raise ValueError('Retain the exact source text and request object')
    table = io.StringIO(newline='')
    writer = csv.writer(table, lineterminator='\n')
    writer.writerow(['time_h', 'side', 'cp_mg_per_L'])
    values = next(column['values'] for column in result['outputs'] if column['name'] == 'cp')
    writer.writerows(zip(result['times'], result['sides'], values))
    csv_bytes = table.getvalue().encode('utf-8')
    source_bytes, request_bytes, result_bytes = source.encode('utf-8'), encode(request), encode(result)
    manifest = {'schema': 'qspquest.table-manifest/v1', 'producer': producer,
        'table': {'filename': 'first-run.csv', 'sha256': sha(csv_bytes), 'bytes': len(csv_bytes), 'rows': 4,
            'columns': [{'name': 'time_h', 'unit': 'h', 'kind': 'time', 'native_name': None, 'meaning': 'Observation time'},
                        {'name': 'side', 'unit': None, 'kind': 'event_side', 'native_name': None, 'meaning': 'Post-dose observation'},
                        {'name': 'cp_mg_per_L', 'unit': 'mg/L', 'kind': 'output', 'native_name': 'cp', 'meaning': 'Synthetic compartment concentration'}]},
        'events': {'rule': 'nondecreasing_time;equal_time_requires_pre_then_post', 'pre': 0, 'post': 4, 'event_pairs': 0},
        'model': {'id': MODEL_ID, 'version': None, 'archive_sha256': None,
            'provenance': {'synthetic': True, 'source_file': 'model.qsp', 'source_sha256': sha(source_bytes),
                           'request_file': 'request.json', 'request_sha256': sha(request_bytes),
                           'result_file': 'result.json', 'result_sha256': sha(result_bytes)}},
        'engine_identity': result['identity'],
        'inputs': {'effective_request': request, 'selected_outputs': ['cp'],
                   'effective_request_hash': result.get('effective_request_hash'), 'scenario_id': None, 'modified': None},
        'qualification': {'qualified': False, 'status': 'not_assessed', 'scope': 'current_engine_and_effective_request',
            'unit_status': 'unqualified', 'coordinates': 'synthetic_declared_units',
            'recorded_dimensional_qualification': None, 'recorded_availability': None}}
    files = {'model.qsp': source_bytes, 'request.json': request_bytes, 'result.json': result_bytes,
             'first-run.csv': csv_bytes, 'first-run.csv.manifest.json': encode(manifest), 'checks.json': encode(checks)}
    destination = Path(directory).absolute()
    destination.mkdir(mode=0o700, parents=False, exist_ok=False)
    for name, raw in files.items():
        with (destination / name).open('xb') as stream:
            stream.write(raw)
    return {'directory': str(destination), 'producer': producer, 'checks': checks,
            'files': {name: {'bytes': len(raw), 'sha256': sha(raw)} for name, raw in files.items()}}
