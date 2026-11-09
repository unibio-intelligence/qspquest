"""Save simulation results as table bundles with replay inputs and table metadata.

``write_bundle`` saves the bounded synthetic first-run result and checks it against its
analytic solution. ``write_table`` saves any simulation result. Both write
``qspquest.table-manifest/v1`` sidecars that ``docs/examples/table_bundle.py`` reads back.
This module uses only the standard library. Checksums identify bytes; they do not
authenticate or qualify a producer.
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


RULE = 'nondecreasing_time;equal_time_requires_pre_then_post'


def _safe_name(name):
    if not isinstance(name, str) or not name or len(name) > 120 or any(c in name for c in '/\\\0') or name.startswith('.'):
        raise ValueError('Bundle name must be a plain file stem')
    return name


def write_table(directory, result, source, request, *, name='trajectory', model_id=None,
                model_version=None, outputs=None, producer='python', scenario_id=None,
                modified=None, provenance=None):
    """Write any simulation result as a table bundle that ``table_bundle.read_table`` accepts.

    The new private directory receives ``<name>.csv`` and ``<name>.csv.manifest.json``
    (``qspquest.table-manifest/v1``) together with the exact model source, request and
    result. ``outputs`` selects columns in order; the default keeps every output. The
    manifest records units, event sides, engine identity and the effective request; it
    never asserts qualification. Existing paths are refused rather than replaced.
    """
    if producer not in ('cli', 'python', 'mcp'):
        raise ValueError('Producer must be cli, python or mcp')
    if not isinstance(source, str) or not isinstance(request, dict):
        raise ValueError('Retain the exact source text and request object')
    if not isinstance(result, dict) or not isinstance(result.get('outputs'), list):
        raise ValueError('A complete native result with outputs is required')
    times, sides = result.get('times'), result.get('sides')
    if (not isinstance(times, list) or not isinstance(sides, list) or len(times) != len(sides)
            or any(type(t) not in (int, float) or not math.isfinite(t) for t in times)
            or any(s not in ('pre', 'post') for s in sides)):
        raise ValueError('Result times and event sides must be finite and paired')
    time_unit = result.get('time_unit')
    if not isinstance(time_unit, str) or not time_unit:
        raise ValueError('Result requires its time unit')
    available = {column.get('name'): column for column in result['outputs'] if isinstance(column, dict)}
    selected = list(available) if outputs is None else list(outputs)
    if not selected or len(set(selected)) != len(selected) or any(o not in available for o in selected):
        raise ValueError('Selected outputs must be unique names present in the result')
    columns = [available[o] for o in selected]
    for column in columns:
        values = column.get('values')
        if (not isinstance(values, list) or len(values) != len(times)
                or any(type(v) not in (int, float) or not math.isfinite(v) for v in values)):
            raise ValueError(f"Output {column.get('name')} requires one finite value per row")
    counts, pairs = {'pre': 0, 'post': 0}, 0
    for index, (time, side) in enumerate(zip(times, sides)):
        counts[side] += 1
        if index:
            if time < times[index - 1]:
                raise ValueError('Result times must be nondecreasing')
            if time == times[index - 1]:
                if (sides[index - 1], side) != ('pre', 'post') or (index > 1 and times[index - 2] == time):
                    raise ValueError('Equal-time rows must be exactly pre then post')
                pairs += 1
    stem = _safe_name(name)
    time_column = 'time_' + time_unit.replace('/', '_per_').replace('.', '_')
    table = io.StringIO(newline='')
    writer = csv.writer(table, lineterminator='\n')
    writer.writerow([time_column, 'side'] + selected)
    for row, (time, side) in enumerate(zip(times, sides)):
        writer.writerow([repr(float(time)), side] + [repr(float(c['values'][row])) for c in columns])
    csv_bytes = table.getvalue().encode('utf-8')
    source_bytes, request_bytes, result_bytes = source.encode('utf-8'), encode(request), encode(result)
    manifest = {
        'schema': 'qspquest.table-manifest/v1', 'producer': producer,
        'table': {'filename': stem + '.csv', 'sha256': sha(csv_bytes), 'bytes': len(csv_bytes),
                  'rows': len(times),
                  'columns': [{'name': time_column, 'unit': time_unit, 'kind': 'time', 'native_name': None, 'meaning': 'Observation time'},
                              {'name': 'side', 'unit': None, 'kind': 'event_side', 'native_name': None, 'meaning': 'Pre- or post-event value'}]
                  + [{'name': c['name'], 'unit': c.get('unit'), 'kind': 'output', 'native_name': c['name'], 'meaning': None}
                     for c in columns]},
        'events': {'rule': RULE, 'pre': counts['pre'], 'post': counts['post'], 'event_pairs': pairs},
        'model': {'id': model_id or result.get('model_id'), 'version': model_version, 'archive_sha256': None,
                  'provenance': {**(provenance or {}), 'source_file': 'model.qsp', 'source_sha256': sha(source_bytes),
                                 'request_file': 'request.json', 'request_sha256': sha(request_bytes),
                                 'result_file': 'result.json', 'result_sha256': sha(result_bytes)}},
        'engine_identity': result.get('identity'),
        'inputs': {'effective_request': request, 'selected_outputs': selected,
                   'effective_request_hash': result.get('effective_request_hash'),
                   'scenario_id': scenario_id, 'modified': modified},
        'qualification': {'qualified': False, 'status': 'not_assessed', 'scope': 'current_engine_and_effective_request',
                          'unit_status': 'unqualified', 'coordinates': 'declared_model_units',
                          'recorded_dimensional_qualification': None, 'recorded_availability': None}}
    files = {'model.qsp': source_bytes, 'request.json': request_bytes, 'result.json': result_bytes,
             stem + '.csv': csv_bytes, stem + '.csv.manifest.json': encode(manifest)}
    destination = Path(directory).absolute()
    destination.mkdir(mode=0o700, parents=False, exist_ok=False)
    for filename, raw in files.items():
        with (destination / filename).open('xb') as stream:
            stream.write(raw)
    return {'directory': str(destination), 'table': str(destination / (stem + '.csv')),
            'producer': producer, 'rows': len(times),
            'files': {filename: {'bytes': len(raw), 'sha256': sha(raw)} for filename, raw in files.items()}}
