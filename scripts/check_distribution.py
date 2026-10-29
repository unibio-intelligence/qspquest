"""Check installation archive bytes and boundaries against the selected source.

This checks packaging integrity, not installed behavior, signatures or scientific
qualification. Run from the complete matching source checkout before installation.
"""

import argparse
import base64
import csv
from email.parser import BytesParser
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import stat
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = Path('bindings/python/python/qspquest')
# Installation inputs mirror the sdist include policy in pyproject.toml.
SOURCE_PATTERNS = (
    'pyproject.toml', 'Cargo.toml', 'Cargo.lock', '.cargo/config.toml',
    'LICENSE', 'THIRD-PARTY-NOTICES.txt', 'SECURITY.md', 'bindings/python/README.md',
    'crates/*/Cargo.toml', 'crates/*/README.md', 'crates/*/LICENSE', 'crates/*/test-data/**/*', 'crates/*/build.rs', 'crates/*/src/**/*.rs',
    'crates/*/examples/**/*.rs', 'bindings/*/Cargo.toml', 'bindings/mcp/build.rs',
    'bindings/mcp/assets/**/*', 'bindings/*/src/**/*.rs',
    'bindings/r/src/rust/Cargo.toml', 'bindings/r/src/rust/src/**/*.rs',
    'bindings/mcp/examples/parity_audit.rs', 'bindings/wasm/js/engine-worker.mjs',
    'bindings/wasm/LICENSE', 'bindings/wasm/LICENSE-APACHE',
    'bindings/python/python/**/*.py', 'bindings/python/python/**/*.pyi',
    'bindings/python/python/**/py.typed', 'bindings/python/python/qspquest/SECURITY.md',
    'conformance/*.rs', 'conformance/tmdd-cases.json', 'vendor/diffsol/**/*',
    'vendor/DIFFSOL-PROVENANCE.md',
)
MAX_MEMBER_BYTES = 256 * 1024 * 1024
MAX_TOTAL_BYTES = 1024 * 1024 * 1024
MAX_MEMBERS = 10000
FORBIDDEN = {'target', '__pycache__', '.git', '.DS_Store', 'internal'}
NATIVE = re.compile(r'qspquest/_native(?:\.[A-Za-z0-9_-]+)*\.(?:so|pyd)\Z')
BUNDLED_LIBRARY = re.compile(r'qspquest\.libs/[A-Za-z0-9_.+-]+(?:\.so(?:\.[0-9]+)*|\.dylib|\.dll)\Z')


def safe_name(name, directory=False):
    """Require canonical relative archive names, never platform-dependent paths."""
    if not isinstance(name, str) or not name or '\\' in name or ':' in name or '\x00' in name:
        raise ValueError('unsafe archive member path: ' + repr(name))
    clean = name[:-1] if directory and name.endswith('/') else name
    parts = clean.split('/')
    if any(part in ('', '.', '..') for part in parts) or PurePosixPath(clean).is_absolute():
        raise ValueError('unsafe archive member path: ' + name)
    if any(part in FORBIDDEN for part in parts) or name.endswith(('.pyc', '.pyo')):
        raise ValueError('generated or private material in archive: ' + name)
    return clean


def bounded_members(members, size):
    if not members or len(members) > MAX_MEMBERS:
        raise ValueError('archive member count outside packaging limit')
    sizes = [size(member) for member in members]
    if any(value < 0 or value > MAX_MEMBER_BYTES for value in sizes) or sum(sizes) > MAX_TOTAL_BYTES:
        raise ValueError('archive expanded size exceeds packaging limit')


def source_files(root):
    expected = {}
    for pattern in SOURCE_PATTERNS:
        for path in root.glob(pattern):
            if path.is_symlink():
                raise ValueError('symlinked source input: ' + str(path))
            if path.is_file():
                relative = path.relative_to(root).as_posix()
                # Vendored build caches are never installation inputs.
                if any(part in FORBIDDEN for part in path.relative_to(root).parts) or path.suffix in ('.pyc', '.pyo'):
                    continue
                expected[relative] = path
    for name in ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml',
                 'bindings/python/README.md', 'SECURITY.md',
                 'bindings/r/src/rust/Cargo.toml',
                 'bindings/python/python/qspquest/SECURITY.md', 'vendor/diffsol/Cargo.toml'):
        if name not in expected:
            raise ValueError('missing selected source input: ' + name)
    return expected


def python_files(root):
    expected = {}
    for path in (root / PACKAGE).rglob('*'):
        if path.is_symlink():
            raise ValueError('symlinked Python source: ' + str(path))
        if path.is_file() and (path.suffix in ('.py', '.pyi') or path.name in ('py.typed', 'SECURITY.md')):
            expected['qspquest/' + path.relative_to(root / PACKAGE).as_posix()] = path
    if not expected or 'qspquest/SECURITY.md' not in expected:
        raise ValueError('missing selected Python package source or policy')
    return expected


def digest(raw):
    return base64.urlsafe_b64encode(hashlib.sha256(raw).digest()).rstrip(b'=').decode('ascii')


def installation_metadata(raw, root):
    metadata = BytesParser().parsebytes(raw)
    version = re.search(r'^version = "([^"]+)"$',
                        (root / 'pyproject.toml').read_text(), re.MULTILINE).group(1)
    if metadata.is_multipart() or metadata.defects:
        raise ValueError('malformed package metadata')
    if metadata.get_all('Name') != ['qspquest'] or metadata.get_all('Version') != [version]:
        raise ValueError('package metadata does not match selected source')
    content_types = metadata.get_all('Description-Content-Type', [])
    if len(content_types) != 1 or content_types[0].split(';', 1)[0].strip().lower() != 'text/markdown':
        raise ValueError('package metadata missing Markdown installation guide')
    description = metadata.get_payload(decode=True).decode('utf-8').replace('\r\n', '\n')
    if description.strip() != (root / 'bindings/python/README.md').read_text().strip():
        raise ValueError('package installation guide differs from selected source')


def inspect_wheel(path, root=ROOT):
    with zipfile.ZipFile(path) as archive:
        members = archive.infolist()
        bounded_members(members, lambda member: member.file_size)
        files, seen = {}, set()
        for member in members:
            name = safe_name(member.filename, member.is_dir())
            if name in seen:
                raise ValueError('duplicate archive member: ' + name)
            seen.add(name)
            mode = member.external_attr >> 16
            kind = stat.S_IFMT(mode)
            if kind not in (0, stat.S_IFDIR if member.is_dir() else stat.S_IFREG):
                raise ValueError('wheel member must be a regular file or directory: ' + name)
            if member.flag_bits & 1:
                raise ValueError('encrypted wheel member: ' + name)
            if not member.is_dir():
                files[name] = member
        metadata_dirs = {name.split('/')[0] for name in files if '.dist-info/' in name}
        if len(metadata_dirs) != 1:
            raise ValueError('wheel must contain one metadata directory')
        info = metadata_dirs.pop()
        project = (root / 'pyproject.toml').read_text()
        version = re.search(r'^version = "([^"]+)"$', project, re.MULTILINE).group(1)
        if info != 'qspquest-' + version + '.dist-info':
            raise ValueError('wheel metadata directory does not match selected package/version')
        required = python_files(root)
        required.update({info + '/licenses/' + name: root / name for name in ('LICENSE', 'THIRD-PARTY-NOTICES.txt')})
        for name, source in required.items():
            if name not in files:
                raise ValueError('wheel missing released file: ' + name)
            if archive.read(files[name]) != source.read_bytes():
                raise ValueError('wheel file differs from selected source: ' + name)
        for name in ('METADATA', 'WHEEL', 'RECORD'):
            if info + '/' + name not in files:
                raise ValueError('wheel missing metadata: ' + name)
        natives = [name for name in files if NATIVE.fullmatch(name)]
        if len(natives) != 1:
            raise ValueError('wheel must contain exactly one native extension')
        for name in files:
            if name.startswith('qspquest/') and name not in required and name not in natives:
                raise ValueError('unexpected installed Python package file: ' + name)
            if not name.startswith(('qspquest/', info + '/', 'qspquest.libs/')):
                raise ValueError('unexpected wheel installation path: ' + name)
            if name.startswith('qspquest.libs/') and not BUNDLED_LIBRARY.fullmatch(name):
                raise ValueError('unexpected bundled library path: ' + name)
            if name.startswith(info + '/'):
                relative = name[len(info) + 1:]
                if relative not in ('METADATA', 'WHEEL', 'RECORD', 'entry_points.txt',
                                    'top_level.txt', 'licenses/LICENSE',
                                    'licenses/THIRD-PARTY-NOTICES.txt', 'RECORD.jws', 'RECORD.p7s') and not (
                        relative.startswith('sboms/') and relative.endswith('.json')):
                    raise ValueError('unexpected wheel metadata file: ' + name)
        installation_metadata(archive.read(info + '/METADATA'), root)
        wheel = BytesParser().parsebytes(archive.read(info + '/WHEEL'))
        if wheel.get('Root-Is-Purelib', '').lower() != 'false' or not wheel.get_all('Tag'):
            raise ValueError('wheel metadata must declare native platform tags')
        record_name = info + '/RECORD'
        # Signed records need a separate signature verifier; never silently drop them.
        if any(info + '/' + name in files for name in ('RECORD.jws', 'RECORD.p7s')):
            raise ValueError('signed wheel RECORD requires separate signature verification')
        rows = csv.reader(io.StringIO(archive.read(record_name).decode('utf-8'), newline=''))
        records = {}
        for row in rows:
            if len(row) != 3:
                raise ValueError('malformed wheel RECORD row')
            name, checksum, size = row
            safe_name(name)
            if name in records:
                raise ValueError('duplicate wheel RECORD entry: ' + name)
            records[name] = (checksum, size)
        if set(records) != set(files):
            raise ValueError('wheel RECORD does not cover exactly the installed files')
        for name, (checksum, size) in records.items():
            if name == record_name:
                if checksum or size:
                    raise ValueError('wheel RECORD self-entry must omit hash and size')
                continue
            raw = archive.read(files[name])
            if checksum != 'sha256=' + digest(raw) or size != str(len(raw)):
                raise ValueError('wheel RECORD hash or size mismatch: ' + name)
        return len(members)


def inspect_source(path, root=ROOT):
    with tarfile.open(path) as archive:
        members = archive.getmembers()
        bounded_members(members, lambda member: member.size)
        originals, seen = [], set()
        for member in members:
            if not (member.isfile() or member.isdir()):
                raise ValueError('source archive member must be a regular file or directory')
            name = safe_name(member.name, member.isdir())
            if name in seen:
                raise ValueError('duplicate archive member: ' + name)
            seen.add(name)
            originals.append((name, member))
        # Only strip a wrapper if every member shares it and its root Cargo.toml exists.
        possible = originals[0][0].split('/')[0]
        wrapped = possible + '/Cargo.toml' in seen and all(
            name == possible or name.startswith(possible + '/') for name, _ in originals)
        expected = source_files(root)
        files = {}
        for original, member in originals:
            name = original[len(possible) + 1:] if wrapped else original
            if wrapped and original == possible:
                if not member.isdir():
                    raise ValueError('source wrapper must be a directory')
                continue
            if member.isdir():
                if not any(required.startswith(name + '/') for required in expected):
                    raise ValueError('excluded source archive directory: ' + name)
                continue
            if name not in expected and name != 'PKG-INFO':
                raise ValueError('excluded development material in source archive: ' + name)
            raw = archive.extractfile(member).read()
            if name in expected and raw != expected[name].read_bytes():
                raise ValueError('source archive file differs from selected source: ' + name)
            files[name] = raw if name == 'PKG-INFO' else None
        missing = sorted((set(expected) | {'PKG-INFO'}) - set(files))
        if missing:
            raise ValueError('incomplete source archive: ' + ', '.join(missing))
        if 'PKG-INFO' in files:
            installation_metadata(files['PKG-INFO'], root)
        return len(members)


def inspect_directory(directory, root=ROOT):
    records = []
    for path in sorted(directory.iterdir()):
        if path.is_symlink():
            raise ValueError('symlinked artifact: ' + path.name)
        if not path.is_file():
            continue
        before = path.stat()
        if path.suffix == '.whl':
            count = inspect_wheel(path, root)
        elif path.name.endswith('.tar.gz'):
            count = inspect_source(path, root)
        else:
            continue
        checksum = hashlib.sha256()
        with path.open('rb') as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                checksum.update(chunk)
        after = path.stat()
        if path.is_symlink() or any(getattr(before, key) != getattr(after, key)
                                  for key in ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')):
            raise ValueError('artifact changed during inspection: ' + path.name)
        records.append({'file': path.name, 'bytes': after.st_size,
                        'sha256': checksum.hexdigest(), 'files': count})
    if not records:
        raise ValueError('no installation artifacts found')
    return {'schema': 'qspquest.installation-artifacts/v0.1', 'artifacts': records}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    try:
        report = json.dumps(inspect_directory(args.directory), indent=2) + '\n'
        if args.out:
            with args.out.open('x', encoding='utf-8') as stream:
                stream.write(report)
        print(report, end='')
    except (ValueError, OSError, RuntimeError, EOFError, zipfile.BadZipFile,
            tarfile.TarError, csv.Error, UnicodeError) as error:
        parser.exit(1, str(error) + '\n')


if __name__ == '__main__':
    main()
