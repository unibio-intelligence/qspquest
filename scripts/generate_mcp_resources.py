"""Generate offline documentation and the installed Python security policy."""
import argparse
import hashlib
import json
from pathlib import Path
import re
from urllib.parse import urlsplit

from doc_links import LINK, anchors, local_target, prose

GUIDES = {
    'security': 'SECURITY.md',
    'release-verification': 'docs/release-verification.md',
    'language': 'spec/language.md', 'requests': 'spec/run-results.md',
    'model': 'spec/model-ir.md', 'regimens': 'spec/regimens.md',
    'units': 'spec/units.md', 'expressions': 'spec/expressions.md',
    'requirements': 'spec/requirements.md', 'invariants': 'spec/invariants.md',
    'tables': 'docs/tutorials/mcp-result-resources.md',
    'predictive-checks': 'docs/tutorials/parallel-predictive-checks.md',
    'local-jobs': 'docs/tutorials/mcp-local-jobs.md',
    'library-fitting': 'docs/tutorials/library-fitting.md',
    'managed-artifacts': 'docs/tutorials/managed-result-artifacts.md',
    'population-records': 'docs/tutorials/mcp-population-records.md',
    'paired-endpoint-fitting': 'docs/tutorials/paired-endpoint-fitting.md',
}
EXAMPLES = {'synthetic-model': 'conformance/models/synthetic-one-compartment.json',
            'synthetic-run': 'conformance/requests/synthetic-one-compartment.json'}
SCHEMA_ALIASES = {'fit-request': 'fit', 'morris-request': 'morris',
                  'scan-request': 'scan', 'sensitivity-request': 'sensitivity',
                  'scalar-fit-request': 'scalar-fit', 'steady-state-request': 'steady-state',
                  'cycle-iteration-request': 'cycle-iteration'}


def encode(value):
    return json.dumps(value, indent=2, ensure_ascii=False) + '\n'


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def generated(root):
    root = root.resolve()
    assets = Path('bindings/mcp/assets')
    paths = {root / path: 'qspquest://guide/' + name for name, path in GUIDES.items()}
    paths.update({root / path: 'qspquest://examples/' + name for name, path in EXAMPLES.items()})
    paths[root / 'spec/capabilities.json'] = 'qspquest://capability-catalog'
    schemas = sorted((root / 'spec/schema').glob('*.schema.json'))
    for path in schemas:
        name = path.name.removesuffix('.schema.json')
        paths[path] = 'qspquest://schema/' + SCHEMA_ALIASES.get(name, name)
    queue = list(paths)
    for path in queue:
        if path.suffix != '.md':
            continue
        for link in LINK.finditer(prose(path.read_text())):
            target = local_target(root, path, link.group(1))
            if target is None:
                continue
            file, _ = target
            if not file.is_file():
                raise ValueError('An offline resource link must resolve to a file: ' + str(file))
            if file not in paths:
                relative = file.relative_to(root).as_posix()
                if relative.split('/')[0] not in ('spec', 'docs', 'conformance', 'vendor', 'bindings'):
                    raise ValueError('Resource outside supported documentation roots: ' + relative)
                paths[file] = 'qspquest://documentation/files/' + relative
                queue.append(file)
    output, entries = {}, []
    for path, uri in sorted(paths.items(), key=lambda entry: entry[1]):
        source = path.read_text()
        text = source
        if path.suffix == '.md':
            edits = []
            for link in LINK.finditer(prose(source)):
                target = local_target(root, path, link.group(1))
                if target is not None:
                    file, fragment = target
                    edits.append((link.start(1), link.end(1), paths[file] + ('#' + fragment if fragment else '')))
            for start, end, replacement in reversed(edits):
                text = text[:start] + replacement + text[end:]
        relative = path.relative_to(root).as_posix()
        if path in schemas:
            asset = assets / path.name
            mime = 'application/json'
        else:
            asset = assets / 'guides' / relative
            mime = 'text/markdown' if path.suffix == '.md' else 'application/json' if path.suffix == '.json' else 'text/plain'
        output[asset] = text
        entries.append({'uri': uri, 'title': source.splitlines()[0].lstrip('# ') if path.suffix == '.md' else path.name,
                        'mime_type': mime, 'asset': asset.relative_to(assets).as_posix(),
                        'source': relative, 'source_sha256': sha(source), 'content_sha256': sha(text),
                        'anchors': sorted(anchors(source)) if path.suffix == '.md' else []})
    manifest = (root / 'Cargo.toml').read_text()
    workspace = manifest.split('[workspace.package]', 1)[1].split('\n[', 1)[0]
    version = re.search(r'^version\s*=\s*"([^"]+)"', workspace, re.MULTILINE).group(1)
    index = {'schema': 'qspquest.documentation/v1', 'package_version': version,
             'scope': 'Structural model, request, unit, error and source-declared capability guidance; documentation is not model or release qualification.',
             'capabilities_uri': 'qspquest://capabilities',
             'capability_catalog_uri': 'qspquest://capability-catalog',
             'topics': {'model': paths[root / GUIDES['model']], 'request': paths[root / GUIDES['requests']],
                        'security': paths[root / GUIDES['security']],
                        'units': paths[root / GUIDES['units']],
                        'errors': paths[root / GUIDES['language']] + '#formatting-and-diagnostics'},
             'examples': [{'id': 'synthetic-one-compartment', 'model_uri': paths[root / EXAMPLES['synthetic-model']],
                           'request_uri': paths[root / EXAMPLES['synthetic-run']], 'operation': 'simulate',
                           'scope': 'Synthetic analytical concentration example; no clinical model qualification.'}],
             'resources': entries}
    output[assets / 'documentation.json'] = encode(index)
    output[Path('bindings/python/python/qspquest/SECURITY.md')] = (root / 'SECURITY.md').read_text()
    return output


def check(root):
    expected = generated(root)
    stale = [str(path) for path, text in expected.items()
             if not (root / path).is_file() or (root / path).read_text() != text]
    expected_guides = {path for path in expected if 'guides' in path.parts}
    actual_guides = {path.relative_to(root) for path in (root / 'bindings/mcp/assets/guides').rglob('*') if path.is_file()}
    if stale or expected_guides != actual_guides:
        raise ValueError('Embedded resource copies differ from canonical source: ' + str(stale or sorted(actual_guides ^ expected_guides)))
    return len(expected)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        if args.check:
            count = check(args.root.resolve())
        else:
            files = generated(args.root)
            for path, text in files.items():
                destination = args.root / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text(text)
            count = len(files)
        print(json.dumps({'status': 'matched' if args.check else 'generated', 'files': count}))
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, str(error) + '\n')
