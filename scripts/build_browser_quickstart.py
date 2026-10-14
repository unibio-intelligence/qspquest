"""Package the browser quickstart with prebuilt WebAssembly bindings."""
import argparse
import hashlib
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BINDINGS = ('qspquest_wasm.js', 'qspquest_wasm_bg.wasm',
            'qspquest_wasm.d.ts', 'qspquest_wasm_bg.wasm.d.ts')
SOURCES = ('bindings/wasm/quickstart/index.html', 'bindings/wasm/quickstart/app.mjs',
           'bindings/wasm/quickstart/style.css', 'bindings/wasm/quickstart/library.json', 'bindings/wasm/js/index.mjs',
           'bindings/wasm/js/client.mjs', 'bindings/wasm/js/engine-worker.mjs',
           'bindings/wasm/js/index.d.mts', 'bindings/wasm/js/client.d.mts',
           'conformance/models/synthetic-one-compartment.qsp', 'docs/examples/first-run.json', 'examples/synthetic/absorption.qsp',
           'docs/browser.md', 'docs/tutorials/agents-docker.md',
           'bindings/mcp/scripts/quickstart.py', 'docs/site/shared-models.md', 'docs/tutorials/mcp-result-resources.md', 'docs/tutorials/mcp-deployment-profiles.md', 'LICENSE', 'THIRD-PARTY-NOTICES.txt')


def library_entry(root, model_id, version, scenario):
    import json, re
    if not all(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,127}', value or '') for value in (model_id, version, scenario)):
        raise ValueError('Use plain model, version and scenario identifiers')
    catalog = json.loads((root / 'catalog.json').read_bytes())
    matches = [m for m in catalog['models'] if m['id'] == model_id and m['version'] == version]
    if len(matches) != 1:
        raise ValueError('Choose one exact catalog model ID and version')
    row = matches[0]
    archive = (root / row['archive']).resolve()
    if not archive.is_relative_to(root.resolve()) or not archive.is_file():
        raise ValueError('Archive is outside the selected library')
    raw = archive.read_bytes()
    if hashlib.sha256(raw).hexdigest() != row['omex_sha256']:
        raise ValueError('Library archive checksum differs from catalog')
    import io
    with zipfile.ZipFile(io.BytesIO(raw)) as package:
        names = package.namelist()
        import posixpath
        if any(i.flag_bits & 1 or ((i.external_attr >> 16) & 0o170000) == 0o120000 for i in package.infolist()):
            raise ValueError('Linked or encrypted archive members are unsupported')
        if any('\\' in n or posixpath.normpath(n) != n for n in names):
            raise ValueError('Unsafe archive paths')
        if len(names) != len(set(names)) or any(Path(n).is_absolute() or '..' in Path(n).parts for n in names):
            raise ValueError('Unsafe archive inventory')
        if sum(i.file_size for i in package.infolist()) > 50000000:
            raise ValueError('Library archive exceeds browser packaging budget')
        members = {name: package.read(name) for name in names}
    index = json.loads(members['index.json'])
    if set(index['members']) != set(members) - {'index.json', 'manifest.xml'}:
        raise ValueError('Archive member inventory differs')
    if any(hashlib.sha256(members[n]).hexdigest() != h for n, h in index['members'].items()):
        raise ValueError('Library member checksum differs')
    if index.get('profile') != 'ubi.omex.rust-ode/v1':
        raise ValueError('Unsupported library archive profile')
    if index.get('model_id') != model_id or index.get('model_version') != version:
        raise ValueError('Archive model ID or version differs from the selected catalog entry')
    native = index['native_execution']
    if native['schema'] != 'qspquest.models.native-execution/v1' or scenario not in native['scenarios']:
        raise ValueError('Select an available native simulation scenario')
    catalog_scenarios = row.get('scenario_ids', [])
    declared_scenarios = index.get('scenarios', [])
    if not isinstance(catalog_scenarios, list) or not isinstance(declared_scenarios, list) or not all(isinstance(item, dict) for item in declared_scenarios):
        raise ValueError('Catalog and archive scenario inventories must be lists')
    archive_scenarios = [item.get('scenario_id') for item in declared_scenarios]
    if catalog_scenarios.count(scenario) != 1 or archive_scenarios.count(scenario) != 1:
        raise ValueError('Selected scenario must be declared exactly once in the catalog and archive index')
    if scenario in index.get('unavailable_scenario_ids', []):
        raise ValueError('This scenario is unavailable for execution')
    source = members[native['model']].decode()
    if json.loads(source).get('model_id') != model_id:
        raise ValueError('Native model ID differs from the selected catalog entry')
    request = json.loads(members[native['scenarios'][scenario]])
    if request['schema'] != 'qspquest.run/v0.1':
        raise ValueError('Scenario is not a native simulation request')
    # The browser is bounded; model coordinates and all scenario values stay intact.
    request['budgets']['output_values'] = min(request['budgets']['output_values'], 20000)
    request['budgets']['events'] = min(request['budgets']['events'], 10000)
    request['budgets']['solver_callbacks'] = min(request['budgets']['solver_callbacks'], 1000000)
    if len(source.encode()) > 1000000 or len(json.dumps(request).encode()) > 1000000:
        raise ValueError('Model or request exceeds browser input budget')
    legal = {f'library/{model_id}/{version}/' + n: members[n] for n in ('LICENSE', 'LICENSE.source', 'NOTICE') if n in members}
    if not any(name.endswith('/LICENSE') for name in legal):
        raise ValueError('Library model is missing its license')
    return {'name':row['name'], 'id':model_id, 'version':version, 'scenario':scenario,
            'description':row.get('description', ''), 'license':row['license'],
            'archive_sha256':row['omex_sha256'], 'source':source, 'request':request,
            'output_labels': {v['native_name']:k for k,v in native.get('outputs',{}).items()}}, legal


def package(assets, output, library_root=None, model_id=None, version=None, scenario=None):
    files = {}
    for name in SOURCES:
        path = ROOT / name
        if not path.is_file() or path.is_symlink():
            raise ValueError('Required quickstart file is missing: ' + name)
        files[name] = path.read_bytes()
    for name in BINDINGS:
        path = assets / name
        if not path.is_file() or path.is_symlink():
            raise ValueError('Build the browser bindings first; missing: ' + name)
        files['bindings/wasm/pkg/engine/' + name] = path.read_bytes()
    if not files['bindings/wasm/pkg/engine/qspquest_wasm_bg.wasm'].startswith(b'\0asm'):
        raise ValueError('The browser engine is not a WebAssembly module')
    files['index.html'] = b'<!doctype html><html lang="en"><meta charset="utf-8"><meta http-equiv="refresh" content="0;url=bindings/wasm/quickstart/index.html"><title>QSPQuest</title><a href="bindings/wasm/quickstart/index.html">Open the browser quickstart</a></html>\n'
    files['README.md'] = b'''# QSPQuest browser quickstart

Extract this archive and open a terminal in its folder. With Python 3.10+:

```sh
python3 -m http.server 8000 --bind 127.0.0.1
```

On Windows, use `py -m http.server 8000 --bind 127.0.0.1`.
Open http://127.0.0.1:8000 and select Run simulation. No Rust installation is
needed. Keep the terminal running; stop with Ctrl+C. See docs/browser.md for
expected results, common errors and the local-computation boundary.
'''
    if library_root is not None:
        import json
        if not all((model_id, version, scenario)):
            raise ValueError('Library packaging requires model ID, version and scenario')
        entry, legal = library_entry(library_root, model_id, version, scenario)
        files['bindings/wasm/quickstart/library.json'] = (json.dumps([entry], indent=2) + '\n').encode()
        files.update(legal)
    elif any((model_id, version, scenario)):
        raise ValueError('Provide --library-root for a library selection')
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, 'x', compression=zipfile.ZIP_DEFLATED) as archive:
        for name, raw in sorted(files.items()):
            entry = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            entry.external_attr = 0o100644 << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, raw)
    return {'artifact': str(output), 'sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
            'bytes': output.stat().st_size,
            'files': {name: hashlib.sha256(raw).hexdigest() for name, raw in sorted(files.items())}}


if __name__ == '__main__':
    import json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, default=ROOT / 'bindings/wasm/pkg/engine')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--library-root', type=Path)
    parser.add_argument('--library-model')
    parser.add_argument('--library-version')
    parser.add_argument('--library-scenario')
    args = parser.parse_args()
    try:
        print(json.dumps(package(args.assets, args.output, args.library_root, args.library_model, args.library_version, args.library_scenario), indent=2))
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f'Browser bundle failed: {error}\n')
