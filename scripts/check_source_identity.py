"""Verify installed-package doctor identities against the exact engine source bytes."""
import argparse
import hashlib
import json
import re
from pathlib import Path

DIRECTORIES = ('crates/qspquest-core/src', 'crates/qspquest-modeling/src', 'bindings/wasm/src')
FIXED = ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'crates/qspquest-core/Cargo.toml',
         'crates/qspquest-modeling/Cargo.toml', 'bindings/wasm/Cargo.toml', 'bindings/mcp/src/core/compute.rs', 'crates/qspquest-modeling/build.rs')


def locked_backend_version(text):
    """Read the generated Cargo.lock package identity with Python 3.10 stdlib."""
    packages = re.split(r'(?m)^\[\[package\]\][ \t]*$', text)[1:]
    backends = [package for package in packages
                if re.search(r'(?m)^name[ \t]*=[ \t]*"diffsol"[ \t]*$', package)]
    if len(backends) != 1:
        raise ValueError('Exactly one locked Diffsol package is required')

    def field(name):
        values = re.findall(r'(?m)^' + name + r'[ \t]*=([^\r\n]*)$', backends[0])
        value = re.fullmatch(r'[ \t]*"([^"\\\r\n]*)"[ \t]*', values[0]) if len(values) == 1 else None
        if value is None:
            raise ValueError('Locked Diffsol requires one valid ' + name)
        return value.group(1)

    field('name')
    version = field('version')
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?', version):
        raise ValueError('Locked Diffsol version is invalid')
    source = re.fullmatch(
        r'git\+https://github\.com/martinjrobins/diffsol\?rev=([0-9a-f]{40})#\1',
        field('source'))
    if source is None:
        raise ValueError('Diffsol must identify its exact upstream Git revision')
    return version + '+git:' + source.group(1)


def source_hashes(root):
    root = Path(root).resolve()
    if (root / 'public-candidate/Cargo.toml').is_file():
        root = root / 'public-candidate'
    source = [path for directory in DIRECTORIES for path in (root / directory).rglob('*.rs')]
    source += [root / name for name in FIXED]
    source_hash = hashlib.sha256()
    for path in sorted(source, key=lambda p: p.relative_to(root).parts):
        if path.is_symlink():
            raise ValueError('Source identity requires regular files, not links')
        name = path.relative_to(root).as_posix().encode('utf-8')
        content = path.read_bytes()
        entry = len(name).to_bytes(8, 'big') + name + len(content).to_bytes(8, 'big') + content
        source_hash.update(entry)
    return ('sha256:' + source_hash.hexdigest(),
            locked_backend_version((root / 'Cargo.lock').read_text()))


def verify(root, doctor, target):
    source, backend = source_hashes(root)
    if doctor.get('schema') != 'qspquest.doctor/v0.1' or doctor.get('status') != 'passed':
        raise ValueError('An overall passing QSPQuest doctor report is required')
    checks = doctor.get('checks', [])
    analytic = [row for row in checks if row.get('name') == 'analytic_simulation']
    if len(analytic) != 1 or any(row.get('status') != 'passed' for row in checks):
        raise ValueError('Doctor checks must pass and include one analytic simulation')
    identity = analytic[0].get('detail', {}).get('identity', {})
    if identity.get('compiler_source_sha256') != source:
        raise ValueError('Installed compiler source differs from the selected source bytes')
    if identity.get('target') != target:
        raise ValueError('Installed compiler target differs from the selected artifact target')
    if identity.get('backend_version') != backend:
        raise ValueError('Installed backend differs from the locked upstream revision')
    return {'status': 'verified', 'compiler_source_sha256': source,
            'backend_version': backend, 'target': target}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--doctor', type=Path, required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(verify(args.source, json.loads(args.doctor.read_text(encoding='utf-8')),
                                args.target), indent=2))
    except (ValueError, OSError, TypeError, AttributeError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
