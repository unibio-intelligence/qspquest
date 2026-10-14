"""Verify installed-package doctor identities against the exact engine source bytes."""
import argparse
import hashlib
import json
from pathlib import Path

DIRECTORIES = ('crates/qspquest-core/src', 'crates/qspquest-modeling/src', 'bindings/wasm/src')
FIXED = ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'crates/qspquest-core/Cargo.toml',
         'crates/qspquest-modeling/Cargo.toml', 'bindings/wasm/Cargo.toml', 'bindings/mcp/src/core/compute.rs', 'crates/qspquest-modeling/build.rs',
         'vendor/DIFFSOL-PROVENANCE.md')


def source_hashes(root):
    root = Path(root).resolve()
    source = [path for directory in DIRECTORIES for path in (root / directory).rglob('*.rs')]
    source += [root / name for name in FIXED]
    vendor = [path for path in (root / 'vendor/diffsol').rglob('*') if path.is_file()
              and path.name != 'Cargo.lock'
              and not {'target', '.git'}.intersection(path.relative_to(root).parts)]
    source_hash, vendor_hash = hashlib.sha256(), hashlib.sha256()
    for paths, hashes in ((source, (source_hash,)), (vendor, (source_hash, vendor_hash))):
        for path in sorted(paths, key=lambda p: p.relative_to(root).parts):
            if path.is_symlink():
                raise ValueError('Source identity requires regular files, not links')
            name = path.relative_to(root).as_posix().encode('utf-8')
            content = path.read_bytes()
            entry = len(name).to_bytes(8, 'big') + name + len(content).to_bytes(8, 'big') + content
            for digest in hashes:
                digest.update(entry)
    return 'sha256:' + source_hash.hexdigest(), vendor_hash.hexdigest()


def verify(root, doctor, target):
    source, vendor = source_hashes(root)
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
    if identity.get('backend_version', '').split('+sha256:')[-1] != vendor:
        raise ValueError('Installed backend source differs from the selected vendored source bytes')
    return {'status': 'verified', 'compiler_source_sha256': source,
            'vendored_source_sha256': vendor, 'target': target}


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
