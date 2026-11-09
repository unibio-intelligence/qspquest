"""Validate source-declared workflow capabilities and generate their human matrix.

The catalog describes interface contracts. It cannot qualify a running artifact
or establish convergence of an unexecuted model/request.

The checks are structural: every declared source file exists and contains its
symbol, every example path exists, and the MCP entries equal the tool router.
The catalog stores no source digests, so editing engine code does not require
editing the catalog. A running build reports its own source identity.
"""
import argparse
import itertools
import json
from pathlib import Path
import re


def contract(schema, value, path='$'):
    """Validate the closed keyword subset used by the owned catalog schema.

    This is a project contract checker, not a general JSON Schema validator.
    """
    if 'const' in schema and value != schema['const']:
        raise ValueError(path + ': constant differs')
    if 'enum' in schema and value not in schema['enum']:
        raise ValueError(path + ': unsupported enum value')
    kind = schema.get('type')
    types = {'object': dict, 'array': list, 'string': str, 'boolean': bool, 'integer': int}
    if kind and type(value) is not types[kind]:
        raise ValueError(path + ': expected ' + kind)
    if kind == 'object':
        if len(value) < schema.get('minProperties', 0):
            raise ValueError(path + ': empty required map')
        if set(schema.get('required', [])) - value.keys():
            raise ValueError(path + ': missing required fields')
        properties = schema.get('properties', {})
        for key, item in value.items():
            child = properties.get(key, schema.get('additionalProperties', {}))
            if child is False:
                raise ValueError(path + ': unknown field ' + key)
            if isinstance(child, dict):
                contract(child, item, path + '.' + key)
    elif kind == 'array':
        if not schema.get('minItems', 0) <= len(value) <= schema.get('maxItems', len(value)):
            raise ValueError(path + ': array length outside contract')
        if schema.get('uniqueItems') and len({json.dumps(v, sort_keys=True) for v in value}) != len(value):
            raise ValueError(path + ': duplicate array entry')
        for i, item in enumerate(value):
            contract(schema.get('items', {}), item, path + '[' + str(i) + ']')
    elif kind == 'string':
        if len(value) < schema.get('minLength', 0) or 'pattern' in schema and not re.search(schema['pattern'], value):
            raise ValueError(path + ': invalid string')
    elif kind == 'integer' and value < schema.get('minimum', value):
        raise ValueError(path + ': integer below minimum')


def file(root, name):
    path = root / name
    if not path.resolve().is_relative_to(root.resolve()) or path.is_symlink() or not path.is_file():
        raise ValueError('Capability source/example is absent or unsafe: ' + name)
    return path


def validate(root, catalog=None):
    root = root.resolve()
    catalog = catalog if catalog is not None else json.loads((root / 'spec/capabilities.json').read_text())
    schema = json.loads((root / 'spec/schema/capability-catalog.schema.json').read_text())
    contract(schema, catalog)
    if set(catalog['operation_labels']) != {e['operation'] for e in catalog['entries']}:
        raise ValueError('Capability operation labels differ from declared entries')
    version = re.search(r'^version\s*=\s*"([^"]+)"', (root / 'Cargo.toml').read_text(), re.M).group(1)
    if catalog['package_version'] != version:
        raise ValueError('Capability package version is stale')
    router = file(root, 'bindings/mcp/src/server.rs').read_text()
    tools = set(re.findall(r'#\[tool\b.*?\]\s*async fn (\w+)', router, re.S))
    if tools != {e['api'] for e in catalog['entries'] if e['interface'] == 'mcp'}:
        raise ValueError('MCP catalog differs from source-declared tool router')
    examples = {}
    for example in catalog['examples']:
        if example['id'] in examples:
            raise ValueError('Duplicate capability example ID')
        examples[example['id']] = example
        for name in example['paths']:
            file(root, name)
    for entry in catalog['entries']:
        if entry['symbol'] not in file(root, entry['source']).read_text():
            raise ValueError('Capability binding symbol is absent: ' + entry['api'])
        if set(entry['example_ids']) - examples.keys():
            raise ValueError('Capability advertises an absent example ID')
        if {'diffsol_rosenbrock23', 'diffsol_rodas5p'}.intersection(entry['solvers']):
            raise ValueError('Capability advertises an unavailable solver')
        if entry['interface'] == 'wasm_worker' and (entry['operation'] != 'simulate' or entry['solvers'] != ['diffsol_bdf']):
            raise ValueError('Public Worker contract exposes BDF simulation only')
        if entry['interface'] == 'mcp' and (entry['operation'] in ('steady_state', 'morris_screening') or entry['api'] == 'analyze') and 'analysis' not in entry['required_features']:
            raise ValueError('MCP analysis tools require analysis')
        if entry['interface'] == 'mcp' and entry['operation'] in ('preview_job', 'submit_simulation', 'submit_population_fit', 'submit_scan', 'get_job', 'wait_for_job', 'get_artifact') and 'managed' not in entry['required_features']:
            raise ValueError('MCP governed tools require managed')
        if entry['interface'] == 'wasm_sandbox' and 'sandbox-abi' not in entry['required_features']:
            raise ValueError('Raw WASM ABI requires sandbox-abi')
    rows = expand(catalog)
    if len({tuple(row[k] for k in catalog['key_dimensions']) for row in rows}) != len(rows):
        raise ValueError('Duplicate expanded capability tuple')
    return catalog


def expand(catalog):
    rows = []
    for entry in catalog['entries']:
        for estimator, solver, target, mode in itertools.product(entry['estimators'], entry['solvers'], entry['targets'], entry['modes']):
            rows.append({'operation': entry['operation'], 'interface': entry['interface'],
                         'estimator': estimator, 'solver': solver, 'target': target,
                         'execution_mode': mode, 'implemented': entry['implemented'],
                         'declared_exposure': entry['exposed'], 'required_features': entry['required_features'],
                         'requirements': entry['requirements'], 'limits': entry['limits'],
                         'example_ids': entry['example_ids'], 'evidence_status': entry['evidence_status'],
                         'evidence_ids': entry['evidence_ids']})
    return rows


def matrix(catalog):
    interfaces = ['rust', 'python', 'r', 'cli', 'wasm_direct', 'wasm_worker', 'wasm_sandbox', 'mcp']
    titles = ['Rust', 'Python', 'R', 'CLI', 'Direct WASM', 'Browser Worker', 'Sandbox ABI', 'MCP']
    labels = catalog['operation_labels']
    operations = sorted({e['operation'] for e in catalog['entries']}, key=lambda op: labels[op].lower())
    lines = ['# Supported interfaces', '',
             'This table is generated from the [capability catalog](../../spec/capabilities.json).',
             '"Declared" means the interface exposes the workflow, "Requires" names the build',
             'feature it needs, and a dash means it is not exposed. Whether a run succeeds still',
             'depends on the build, deployment, model and request. Nothing here is a scientific',
             'qualification.', '',
             '| Workflow | ' + ' | '.join(titles) + ' |',
             '| --- | ' + ' | '.join('---' for _ in interfaces) + ' |']
    for operation in operations:
        cells = []
        for interface in interfaces:
            matching = [e for e in catalog['entries'] if e['operation'] == operation and
                        (e['interface'] == interface or interface == 'python' and e['interface'].startswith('python_'))]
            cells.append('; '.join(('Requires ' + ', '.join('`' + f + '`' for f in e['required_features']))
                                   if e['required_features'] else 'Declared' for e in matching) or '—')
        lines.append('| ' + labels[operation] + ' | ' + ' | '.join(cells) + ' |')
    lines.extend(['', 'The Python column combines the native binding and the analysis workflows',
                  '(`python_analysis`), which run in the native engine and need no extra. The',
                  '`saem` fit request and the analysis `fit_population` run the same SAEM',
                  'engine. Direct WASM browser classes are absent in `sandbox-abi`',
                  'builds, and the browser Worker runs BDF simulation only.', '',
                  '## Solvers', '',
                  'Native simulation offers BDF, TSIT45, ESDIRK34, TR-BDF2 and analytic linear PK.',
                  'Sensitivities and sensitivity-based fits accept BDF. Periodic linear steady state requires `analytic_linear_pk`',
                  'and a compatible `linear_pk` declaration; bounded cycle iteration requires BDF.',
                  'See the [fitting solver guide](fitting-solvers.md),',
                  '[Python guide](../PYTHON-USER-GUIDE.md) and [MCP guide](../../bindings/mcp/README.md)',
                  'for inputs.', '',
                  '## Check a workflow in MCP', '',
                  '`qspquest://capabilities` reports the active tools and limits, and',
                  '`qspquest://capability-registry` the server\'s target and mode. `get_capability`',
                  'takes the six key fields (operation, interface, estimator, solver, target and',
                  'execution mode), optionally with a model, version and scenario, and lists',
                  'blockers. `executable: null` means the request still needs normal admission.',
                  'Hosted MCP runs user-supplied models only with a sandbox build. Governed records',
                  'describe the transport; platform readiness needs an authorized preview.', '',
                  '## Qualification', '',
                  'Catalog records have scientific status `untested`. They describe what is',
                  'implemented and exposed, not whether a release, model or study is qualified. A',
                  'passing synthetic example covers only its tested numerical check.', ''])
    return '\n'.join(lines)


def check(root):
    catalog = validate(root)
    expected = matrix(catalog)
    if (root / 'docs/site/capabilities.md').read_text() != expected:
        raise ValueError('Generated capability matrix is stale')
    return {'status': 'matched', 'entries': len(catalog['entries']),
            'tuples': len(expand(catalog)), 'source_files': len({e['source'] for e in catalog['entries']}),
            'scientific_qualification': 'not_assessed'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        root = args.root.resolve()
        if args.check:
            report = check(root)
        else:
            catalog = validate(root)
            (root / 'docs/site/capabilities.md').write_text(matrix(catalog))
            report = {'status': 'generated', 'entries': len(catalog['entries']), 'tuples': len(expand(catalog))}
        print(json.dumps(report, indent=2))
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, str(error) + '\n')
