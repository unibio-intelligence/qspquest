"""Verify exact release bytes, build identity and signed SBOM content with gh."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import subprocess

SLSA = 'https://slsa.dev/provenance/v1'
BUILD_TYPE = 'https://actions.github.io/buildtypes/workflow/v1'
MAX_JSON = 64 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'Duplicate JSON field: ' + key)
        result[key] = value
    return result


def decode(data):
    require(len(data.encode() if isinstance(data, str) else data) <= MAX_JSON,
            'Attestation JSON exceeds the supported size')
    def invalid(value):
        raise ValueError('Nonfinite JSON number: ' + value)
    def number(value):
        result = float(value)
        require(math.isfinite(result), 'Nonfinite JSON number')
        return result
    return json.loads(data, object_pairs_hook=unique, parse_constant=invalid, parse_float=number)


def fingerprint(path):
    require(path.is_file() and not path.is_symlink(), 'Expected a regular input file: ' + str(path))
    before = path.stat()
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(data)
    after = path.stat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
            == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
            and not path.is_symlink(), 'Input changed during hashing: ' + str(path))
    return digest.hexdigest()


def policy(repository, workflow, source_commit, signer_commit, artifact_sha256, sbom_sha256):
    require(bool(re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository)),
            'Repository must be an explicit GitHub owner/repository')
    require(bool(re.fullmatch(r'\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml', workflow)),
            'Workflow must name one .github/workflows/*.yml or *.yaml file')
    for label, value, length in [('source commit', source_commit, 40), ('signer commit', signer_commit, 40),
                                 ('artifact SHA-256', artifact_sha256, 64), ('SBOM SHA-256', sbom_sha256, 64)]:
        require(bool(re.fullmatch('[0-9a-f]{' + str(length) + '}', value)),
                'Expected an immutable lowercase ' + label)
    return {'repository': repository, 'workflow': workflow, 'source_commit': source_commit,
            'signer_commit': signer_commit, 'artifact_sha256': artifact_sha256,
            'sbom_sha256': sbom_sha256}


def sbom_type(sbom):
    require(type(sbom) is dict, 'SBOM must be a JSON object')
    if 'spdxVersion' in sbom:
        require(type(sbom['spdxVersion']) is str and bool(re.fullmatch(r'SPDX-2\.[0-9]+', sbom['spdxVersion']))
                and sbom.get('SPDXID') == 'SPDXRef-DOCUMENT', 'Expected an SPDX 2.x JSON document')
        return 'https://spdx.dev/Document/v' + sbom['spdxVersion'].removeprefix('SPDX-')
    require(sbom.get('bomFormat') == 'CycloneDX' and type(sbom.get('specVersion')) is str
            and bool(re.fullmatch(r'1\.[0-9]+', sbom['specVersion']))
            and type(sbom.get('serialNumber')) is str and bool(sbom['serialNumber']),
            'Expected an SPDX 2.x or CycloneDX 1.x JSON SBOM')
    return 'https://cyclonedx.org/bom'


def command(gh, artifact, bundle, expected, predicate_type):
    return [str(gh), 'attestation', 'verify', artifact, '--hostname', 'github.com',
            '--repo', expected['repository'], '--bundle', str(bundle), '--format', 'json',
            '--digest-alg', 'sha256', '--predicate-type', predicate_type,
            '--signer-repo', expected['repository'],
            '--signer-workflow', expected['repository'] + '/' + expected['workflow'],
            '--signer-digest', expected['signer_commit'], '--source-digest', expected['source_commit'],
            '--cert-oidc-issuer', 'https://token.actions.githubusercontent.com',
            '--deny-self-hosted-runners']


def statements(verified, expected, predicate_type):
    require(type(verified) is list and bool(verified), 'No verified attestations were returned')
    result = []
    for entry in verified:
        require(type(entry) is dict and type(entry.get('verificationResult')) is dict,
                'Unexpected GitHub CLI verification output')
        statement = entry['verificationResult'].get('statement')
        require(type(statement) is dict and statement.get('predicateType') == predicate_type,
                'Verified attestation predicate type differs from the requested policy')
        subjects = statement.get('subject')
        require(type(subjects) is list and any(type(s) is dict and type(s.get('digest')) is dict
                and s['digest'].get('sha256') == expected['artifact_sha256'] for s in subjects),
                'Verified attestation does not name the exact artifact digest')
        require(type(statement.get('predicate')) is dict, 'Verified predicate is not an object')
        result.append(statement['predicate'])
    return result


def provenance_matches(predicate, expected):
    def mapping(value):
        require(type(value) is dict, 'Build provenance contains malformed nested fields')
        return value
    definition = mapping(predicate.get('buildDefinition', {}))
    workflow = mapping(mapping(definition.get('externalParameters', {})).get('workflow', {}))
    ref = workflow.get('ref')
    require(type(ref) is str and bool(ref) and not any(ord(c) < 32 for c in ref),
            'Build provenance lacks a source ref')
    require(definition.get('buildType') == BUILD_TYPE
            and workflow.get('repository') == 'https://github.com/' + expected['repository']
            and workflow.get('path') == expected['workflow'], 'Build provenance repository/workflow differs')
    dependencies = definition.get('resolvedDependencies')
    require(type(dependencies) is list and any(type(d) is dict
            and d.get('uri') == 'git+https://github.com/' + expected['repository'] + '@' + ref
            and d.get('digest') == {'gitCommit': expected['source_commit']} for d in dependencies),
            'Build provenance does not resolve the exact source commit')
    require(mapping(mapping(definition.get('internalParameters', {})).get('github', {})).get('runner_environment') == 'github-hosted',
            'Build provenance must declare a GitHub hosted runner')
    builder = mapping(mapping(predicate.get('runDetails', {})).get('builder', {})).get('id')
    prefix = 'https://github.com/' + expected['repository'] + '/' + expected['workflow'] + '@'
    require(type(builder) is str and builder.startswith(prefix) and len(builder) > len(prefix),
            'Build provenance builder differs from the expected workflow')


def sbom_matches(predicate, sbom):
    # JSON equality must distinguish booleans from numbers and retain array order.
    canonical = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)
    require(canonical(predicate) == canonical(sbom), 'Signed SBOM content differs from the supplied SBOM')


def verify(args):
    expected = policy(args.repository, args.workflow, args.source_commit, args.signer_commit,
                      args.sha256, args.sbom_sha256)
    gh = shutil.which('gh')
    require(gh is not None, 'GitHub CLI gh is required; use a version supporting the documented attestation policy flags')
    gh = Path(gh).resolve()
    paths = [gh, args.provenance_bundle, args.sbom_bundle, args.sbom, Path(__file__)]
    if args.artifact:
        paths.append(args.artifact)
        require(fingerprint(args.artifact) == args.sha256, 'Artifact SHA-256 differs from the trusted release checksum')
        artifact = str(args.artifact.absolute())
    else:
        require(bool(re.fullmatch(r'oci://[A-Za-z0-9.:_-]+/[A-Za-z0-9_./-]+@sha256:' + args.sha256, args.image)),
                'Image must be an exact oci://registry/path@sha256:digest matching the trusted release checksum')
        artifact = args.image
    inputs = {str(p.absolute()): fingerprint(p) for p in paths}
    require(fingerprint(args.sbom) == args.sbom_sha256, 'SBOM SHA-256 differs from the trusted release checksum')
    require(args.sbom.stat().st_size <= 16 * 1024 * 1024, 'SBOM exceeds the supported 16 MiB size')
    sbom = decode(args.sbom.read_bytes())
    predicate_type = sbom_type(sbom)
    # A new evidence directory preserves every verifier refusal without replacing old evidence.
    args.out.mkdir(parents=True, exist_ok=False)
    report = {'schema': 'qspquest.release-attestation-verification/v1', 'status': 'failed',
              'policy': expected, 'input_sha256': inputs, 'artifact': artifact, 'checks': [],
              'scientific_qualification': 'not_assessed', 'dependency_inventory_completeness': 'not_assessed'}
    try:
        for name, bundle, kind in [('provenance', args.provenance_bundle, SLSA),
                                   ('sbom', args.sbom_bundle, predicate_type)]:
            cmd = command(gh, artifact, bundle.absolute(), expected, kind)
            try:
                result = subprocess.run(cmd, capture_output=True, text=True, encoding='utf-8', timeout=60)
                (args.out / (name + '.stdout')).write_text(result.stdout)
                (args.out / (name + '.stderr')).write_text(result.stderr)
            except subprocess.TimeoutExpired as error:
                for suffix, value in [('stdout', error.stdout), ('stderr', error.stderr)]:
                    (args.out / (name + '.' + suffix)).write_text(value.decode(errors='replace') if isinstance(value, bytes) else value or '')
                raise ValueError('GitHub CLI ' + name + ' verification timed out') from error
            require(result.returncode == 0, 'GitHub CLI ' + name + ' signature/identity verification failed; see retained output')
            predicates = statements(decode(result.stdout), expected, kind)
            for predicate in predicates:
                if name == 'provenance':
                    provenance_matches(predicate, expected)
                else:
                    sbom_matches(predicate, sbom)
            report['checks'].append({'type': name, 'predicate_type': kind, 'verified_attestations': len(predicates),
                                     'command': cmd, 'status': 'verified'})
        require(all(fingerprint(Path(p)) == digest for p, digest in inputs.items()),
                'Artifact, SBOM, bundle, verifier or script changed during verification')
        report['status'] = 'verified'
    except (ValueError, KeyError, TypeError, OSError, RecursionError) as error:
        report['error'] = str(error)
    with (args.out / 'report.json').open('x') as stream:
        json.dump(report, stream, indent=2)
        stream.write('\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    artifact = parser.add_mutually_exclusive_group(required=True)
    artifact.add_argument('--artifact', type=Path)
    artifact.add_argument('--image')
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--sbom-sha256', required=True)
    parser.add_argument('--provenance-bundle', type=Path, required=True)
    parser.add_argument('--sbom', type=Path, required=True)
    parser.add_argument('--sbom-bundle', type=Path, required=True)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--workflow', required=True)
    parser.add_argument('--source-commit', required=True)
    parser.add_argument('--signer-commit', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    try:
        report = verify(args)
    except (ValueError, OSError, TypeError, RecursionError) as error:
        parser.exit(1, str(error) + '\n')
    print(json.dumps({'status': report['status'], 'report': str(args.out / 'report.json'),
                      'scientific_qualification': 'not_assessed'}))
    return int(report['status'] != 'verified')


if __name__ == '__main__':
    raise SystemExit(main())
