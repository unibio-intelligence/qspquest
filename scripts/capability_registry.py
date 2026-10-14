"""Validate source-declared workflow capabilities and generate their human matrix.

The catalog describes interface contracts. It cannot qualify a running artifact
or establish convergence of an unexecuted model/request.
"""
import argparse
import hashlib
import itertools
import json
from pathlib import Path
import re

from check_source_identity import source_hashes


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
    source, vendor = source_hashes(root)
    if (catalog['compiler_source_sha256'], catalog['vendor_source_sha256']) != (source, vendor):
        raise ValueError('Capability compiler/backend source identity is stale')
    for name, sha in catalog['source_snapshot'].items():
        if hashlib.sha256(file(root, name).read_bytes()).hexdigest() != sha:
            raise ValueError('Capability source snapshot is stale: ' + name)
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
        if entry['source'] not in catalog['source_snapshot'] or entry['symbol'] not in file(root, entry['source']).read_text():
            raise ValueError('Capability binding symbol is absent: ' + entry['api'])
        if set(entry['example_ids']) - examples.keys():
            raise ValueError('Capability advertises an absent example ID')
        if entry['interface'] == 'mcp' and 'diffsol_rosenbrock23' in entry['solvers']:
            raise ValueError('MCP withholds Rosenbrock23')
        if entry['interface'] == 'wasm_worker' and (entry['operation'] != 'simulate' or entry['solvers'] != ['diffsol_bdf']):
            raise ValueError('Public Worker contract exposes BDF simulation only')
        if entry['interface'] == 'wasm_direct' and entry['operation'] in ('sensitivities', 'fit_individual', 'fit_population') and 'm0-conformance' not in entry['required_features']:
            raise ValueError('Direct WASM sensitivity/fitting binding requires m0-conformance')
        if entry['interface'] == 'mcp' and entry['operation'] in ('steady_state', 'morris_screening') and 'analysis' not in entry['required_features']:
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
    operations = sorted({e['operation'] for e in catalog['entries']})
    lines = ['# Supported interfaces', '',
             'This matrix is generated from the [capability catalog](../../spec/capabilities.json).',
             'It lists declared interface contracts. Build features, deployment configuration,',
             'model requirements and request admission determine whether a workflow can execute.',
             'Source declarations do not establish scientific qualification for a release.', '',
             '| Workflow | ' + ' | '.join(titles) + ' |',
             '| --- | ' + ' | '.join('---' for _ in interfaces) + ' |']
    for operation in operations:
        cells = []
        for interface in interfaces:
            matching = [e for e in catalog['entries'] if e['operation'] == operation and
                        (e['interface'] == interface or interface == 'python' and e['interface'].startswith('python_'))]
            cells.append('; '.join(('Requires ' + ', '.join('`' + f + '`' for f in e['required_features']))
                                   if e['required_features'] else 'Declared' for e in matching) or '—')
        lines.append('| ' + catalog['operation_labels'][operation] + ' | ' + ' | '.join(cells) + ' |')
    lines.extend(['', 'A dash means the declared interface does not expose that workflow.',
                  '`python_analysis` uses the optional analysis dependencies; it is distinct from',
                  'native Python sensitivity-based fitting. Native and Python SAEM have different',
                  'estimator contracts. `wasm_direct` browser classes are absent with `sandbox-abi`.',
                  'The public Worker contract declares BDF simulation; direct bindings expose their',
                  'documented additional operations and solvers.', '',
                  '## Solvers, estimators and execution modes', '',
                  'The catalog keys each expanded record by operation, interface, estimator, solver,',
                  'target and execution mode. Native simulation has seven explicit solver choices.',
                  'Sensitivities and native sensitivity-based fits accept BDF or Rodas5P.',
                  'Periodic linear steady state requires `analytic_linear_pk`; bounded cycle',
                  'iteration requires BDF. MCP withholds Rosenbrock23. Analytic execution requires',
                  'a compatible `linear_pk` declaration; availability does not validate a model.', '',
                  'Read the [fitting solver guide](fitting-solvers.md), [Python guide](../PYTHON-USER-GUIDE.md)',
                  'and [MCP guide](../../bindings/mcp/README.md) for their complete input contracts.',
                  'MCP hosted user-source execution requires a configured sandbox. Effective limits',
                  'and active tools are reported by `qspquest://capabilities`.', '',
                  'Read `qspquest://capability-registry` for the active MCP target and mode.',
                  'Use `get_capability` with the six key fields and an optional native model/version',
                  'and stored scenario to inspect blockers. `executable: null` means the exact request',
                  'still needs normal tool admission and execution. Other interfaces and targets',
                  'remain unassessed by this server. Governed records describe the transport build;',
                  'platform model readiness and execution target require an authorized preview.', '',
                  '## Qualification', '',
                  'Catalog records carry `untested` scientific status. They describe implementation',
                  'and interface exposure; they do not qualify the current artifact, clinical model',
                  'or study design. Scientific claims require matching release, operation, solver,',
                  'target, model and evidence scope. A successful synthetic example is limited to',
                  'its tested numerical contract. Model unit and fitting readiness remain separate.', ''])
    return '\n'.join(lines)


def check(root):
    catalog = validate(root)
    expected = matrix(catalog)
    if (root / 'docs/site/capabilities.md').read_text() != expected:
        raise ValueError('Generated capability matrix is stale')
    return {'status': 'matched', 'entries': len(catalog['entries']),
            'tuples': len(expand(catalog)), 'source_files': len(catalog['source_snapshot']),
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
