"""Check QSPQuest distribution identities and current public format declarations."""
import argparse
import json
from pathlib import Path
import re

PACKAGES = {
    'crates/qspquest-core/Cargo.toml':'qspquest-core',
    'crates/qspquest-modeling/Cargo.toml':'qspquest-modeling',
    'bindings/cli/Cargo.toml':'qspquest-cli',
    'bindings/mcp/Cargo.toml':'qspquest-mcp',
    'bindings/python/Cargo.toml':'qspquest-python',
    'bindings/wasm/Cargo.toml':'qspquest-wasm',
    'bindings/r/src/rust/Cargo.toml':'qspquest-r',
}


def field(text, section, key):
    """Read one simple quoted string from the checked-in manifest's named section."""
    blocks = re.findall(r'(?ms)^\['+re.escape(section)+r'\]\s*\n(.*?)(?=^\[|\Z)',text)
    if len(blocks) != 1:
        raise ValueError('Expected exactly one manifest section: '+section)
    values = re.findall(r'(?m)^'+re.escape(key)+r'\s*=\s*"([^"\n]+)"\s*(?:#.*)?$',blocks[0])
    if len(values) != 1:
        raise ValueError('Expected exactly one quoted manifest field: '+section+'.'+key)
    return values[0]


def check(root):
    root = Path(root)
    errors = []
    def text(path):
        target = root/path
        if target.is_symlink() or not target.is_file():
            raise ValueError('Required distribution file missing or unsafe: '+path)
        return target.read_text(encoding='utf-8')
    try:
        for path,name in PACKAGES.items():
            if field(text(path),'package','name') != name:
                errors.append(path+': expected package '+name)
        cli = text('bindings/cli/Cargo.toml')
        names = re.findall(r'(?ms)^\[\[bin\]\]\s*\n.*?^name\s*=\s*"([^"\n]+)"',cli)
        if names != ['qspquest']:
            errors.append('CLI binary must be qspquest')
        lock = text('Cargo.lock')
        locked = re.findall(r'(?ms)^\[\[package\]\]\s*\nname\s*=\s*"([^"\n]+)"',lock)
        if any(locked.count(name) != 1 for name in PACKAGES.values()):
            errors.append('Cargo.lock must include exactly one of each supported local package')
        python = text('pyproject.toml')
        for section,key,expected in [('project','name','qspquest'),('tool.maturin','module-name','qspquest._native'),
            ('project.scripts','qspquest-doctor','qspquest.doctor:main'),
            ('project.scripts','qspquest-library','qspquest.library:main')]:
            if field(python,section,key) != expected:
                errors.append(section+'.'+key+': expected '+expected)
        if sorted(path.parent.name for path in (root/'bindings/python/python').glob('*/__init__.py')) != ['qspquest']:
            errors.append('Python package directory must be qspquest')
        if re.findall(r'(?m)^Package:\s*(\S+)\s*$',text('bindings/r/DESCRIPTION')) != ['qspquest']:
            errors.append('R package must be qspquest')
        if not re.search(r'(?m)^useDynLib\(qspquest,\s*\.registration\s*=\s*TRUE\)\s*$',text('bindings/r/NAMESPACE')):
            errors.append('R dynamic-library registration must use qspquest')
        wasm = json.loads(text('bindings/wasm/package.json'))
        if wasm.get('name') != '@qspquest/engine' or 'pkg/engine/qspquest_wasm.js' not in wasm.get('files',[]):
            errors.append('Browser distribution and generated WASM entry must use qspquest')
        if not re.match(r'# QSPQuest\b',text('README.md')):
            errors.append('README product heading must be QSPQuest')
        for path in sorted((root/'spec/schema').glob('*.json')):
            document = json.loads(path.read_text(encoding='utf-8'))
            def check_schema(node):
                if not isinstance(node, dict):
                    return
                properties = node.get('properties', {})
                schema = properties.get('schema', {})
                if '$ref' in schema:
                    prefix = '#/definitions/'
                    reference = schema['$ref']
                    schema = document.get('definitions', {}).get(reference[len(prefix):], {}) if reference.startswith(prefix) else {}
                if 'schema' in properties:
                    values = schema.get('enum', [schema.get('const')])
                    if not values or any(not isinstance(value, str) or not re.fullmatch(r'qspquest\.[a-z0-9-]+/v[0-9]+(?:\.[0-9]+)*', value) for value in values):
                        errors.append(f'{path.relative_to(root)}: document schema must declare a QSPQuest identifier')
                for value in node.values():
                    if isinstance(value, dict):
                        check_schema(value)
                    elif isinstance(value, list):
                        for item in value:
                            check_schema(item)
            check_schema(document)
    except (OSError,ValueError,TypeError) as error:
        errors.append(str(error))
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    errors = check(args.source)
    print(json.dumps({'status':'failed' if errors else 'matched','local_packages':len(PACKAGES),
                      'errors':errors,'current_schema_namespace':'qspquest'}))
    return bool(errors)


if __name__ == '__main__':
    raise SystemExit(main())
