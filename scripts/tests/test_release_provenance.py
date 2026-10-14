"""Release identity/content refusal fixtures; real signatures need their own gate."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import verify_release_provenance as gate


class ReleaseProvenanceTests(unittest.TestCase):
    def expected(self):
        return gate.policy('release-owner/qspquest', '.github/workflows/release.yml',
                           '1' * 40, '2' * 40, 'a' * 64, 'b' * 64)

    def provenance(self):
        return {'buildDefinition': {'buildType': gate.BUILD_TYPE,
                    'externalParameters': {'workflow': {'ref': 'refs/tags/v0.1.0',
                        'repository': 'https://github.com/release-owner/qspquest',
                        'path': '.github/workflows/release.yml'}},
                    'internalParameters': {'github': {'runner_environment': 'github-hosted'}},
                    'resolvedDependencies': [{'uri': 'git+https://github.com/release-owner/qspquest@refs/tags/v0.1.0',
                                              'digest': {'gitCommit': '1' * 40}}]},
                'runDetails': {'builder': {'id': 'https://github.com/release-owner/qspquest/.github/workflows/release.yml@refs/tags/v0.1.0'}}}

    def output(self, expected, kind, predicate):
        return [{'verificationResult': {'statement': {'predicateType': kind,
                  'subject': [{'name': 'release.bin', 'digest': {'sha256': expected['artifact_sha256']}}],
                  'predicate': predicate}}}]

    def test_policy_requires_immutable_source_signer_and_download_digests(self):
        values = list(self.expected().values())
        for index, replacement in [(0, 'owner/repo/other'), (1, '../release.yml'),
                                   (2, 'main'), (3, 'v0.1.0'), (4, 'abc'), (5, 'B' * 64)]:
            wrong = values.copy()
            wrong[index] = replacement
            with self.assertRaises(ValueError):
                gate.policy(*wrong)
        cmd = gate.command('gh', 'release.bin', 'bundle.json', self.expected(), gate.SLSA)
        for flag, value in [('--repo', 'release-owner/qspquest'),
                            ('--signer-workflow', 'release-owner/qspquest/.github/workflows/release.yml'),
                            ('--source-digest', '1' * 40), ('--signer-digest', '2' * 40),
                            ('--cert-oidc-issuer', 'https://token.actions.githubusercontent.com')]:
            self.assertEqual(cmd[cmd.index(flag) + 1], value)
        self.assertIn('--deny-self-hosted-runners', cmd)

    def test_verified_statement_still_requires_the_requested_digest_and_predicate(self):
        expected = self.expected()
        valid = self.output(expected, gate.SLSA, self.provenance())
        self.assertEqual(gate.statements(valid, expected, gate.SLSA), [self.provenance()])
        wrong_digest = copy.deepcopy(valid)
        wrong_digest[0]['verificationResult']['statement']['subject'][0]['digest']['sha256'] = 'c' * 64
        for wrong in [[], {}, [{'statement': valid}], wrong_digest,
                      self.output(expected, 'https://other/predicate', self.provenance())]:
            with self.assertRaises(ValueError):
                gate.statements(wrong, expected, gate.SLSA)

    def test_provenance_rejects_wrong_source_workflow_builder_and_self_hosted_claims(self):
        valid = self.provenance()
        gate.provenance_matches(valid, self.expected())
        mutations = [(['buildDefinition', 'externalParameters', 'workflow', 'repository'], 'https://github.com/other/repo'),
                     (['buildDefinition', 'externalParameters', 'workflow', 'path'], '.github/workflows/other.yml'),
                     (['buildDefinition', 'internalParameters', 'github', 'runner_environment'], 'self-hosted'),
                     (['buildDefinition', 'resolvedDependencies'], [{'uri': 'wrong', 'digest': {'gitCommit': '1' * 40}}]),
                     (['runDetails', 'builder', 'id'], 'https://github.com/other/repo/workflow.yml@main'),
                     (['buildDefinition', 'externalParameters'], []),
                     (['buildDefinition'], []), (['runDetails', 'builder'], [])]
        for keys, value in mutations:
            wrong = copy.deepcopy(valid)
            parent = wrong
            for key in keys[:-1]:
                parent = parent[key]
            parent[keys[-1]] = value
            with self.assertRaises(ValueError):
                gate.provenance_matches(wrong, self.expected())

    def test_signed_sbom_content_does_not_accept_reordered_arrays_or_changed_types(self):
        sbom = {'spdxVersion': 'SPDX-2.3', 'SPDXID': 'SPDXRef-DOCUMENT', 'packages': ['one', 'two'], 'flag': True}
        self.assertEqual(gate.sbom_type(sbom), 'https://spdx.dev/Document/v2.3')
        gate.sbom_matches(dict(reversed(list(sbom.items()))), sbom)
        for wrong in [dict(sbom, packages=['two', 'one']), dict(sbom, flag=1), dict(sbom, packages=[]),
                      dict(sbom, new_field='extra')]:
            with self.assertRaises(ValueError):
                gate.sbom_matches(wrong, sbom)
        for raw in ['{"a":1,"a":2}', '{"n":NaN}', '{"n":1e999}']:
            with self.assertRaises(ValueError):
                gate.decode(raw)

    def args(self, root):
        names = ['artifact.bin', 'provenance.json', 'sbom-bundle.json', 'sbom.json', 'gh']
        paths = [root / name for name in names]
        for path in paths:
            path.write_text('fixture bytes')
        sbom = {'spdxVersion': 'SPDX-2.3', 'SPDXID': 'SPDXRef-DOCUMENT', 'packages': []}
        paths[3].write_text(json.dumps(sbom))
        args = argparse.Namespace(artifact=paths[0], image=None, provenance_bundle=paths[1],
                   sbom_bundle=paths[2], sbom=paths[3], repository='release-owner/qspquest',
                   workflow='.github/workflows/release.yml', source_commit='1' * 40, signer_commit='2' * 40,
                   sha256=gate.fingerprint(paths[0]), sbom_sha256=gate.fingerprint(paths[3]), out=root / 'evidence')
        return args, paths[4], sbom

    def test_signature_failure_cannot_be_replaced_by_plausible_unsigned_json(self):
        with tempfile.TemporaryDirectory() as folder:
            args, gh, _ = self.args(Path(folder))
            expected = gate.policy(args.repository, args.workflow, args.source_commit, args.signer_commit, args.sha256, args.sbom_sha256)
            result = subprocess.CompletedProcess([], 1, json.dumps(self.output(expected, gate.SLSA, self.provenance())), 'signature rejected')
            with patch.object(gate.shutil, 'which', return_value=str(gh)), patch.object(gate.subprocess, 'run', return_value=result):
                report = gate.verify(args)
            self.assertEqual(report['status'], 'failed')
            self.assertEqual(report['checks'], [])
            self.assertIn('signature/identity', report['error'])
            self.assertEqual((args.out / 'provenance.stderr').read_text(), 'signature rejected')

    def test_input_mutation_after_successful_verifier_calls_fails_and_retains_evidence(self):
        with tempfile.TemporaryDirectory() as folder:
            args, gh, sbom = self.args(Path(folder))
            expected = gate.policy(args.repository, args.workflow, args.source_commit, args.signer_commit, args.sha256, args.sbom_sha256)
            def verified(cmd, **kwargs):
                kind = cmd[cmd.index('--predicate-type') + 1]
                predicate = self.provenance() if kind == gate.SLSA else sbom
                if kind != gate.SLSA:
                    args.artifact.write_text('changed artifact')
                return subprocess.CompletedProcess(cmd, 0, json.dumps(self.output(expected, kind, predicate)), '')
            with patch.object(gate.shutil, 'which', return_value=str(gh)), patch.object(gate.subprocess, 'run', side_effect=verified):
                report = gate.verify(args)
            self.assertEqual(report['status'], 'failed')
            self.assertIn('changed during verification', report['error'])
            self.assertTrue((args.out / 'sbom.stdout').is_file())
            self.assertTrue((args.out / 'report.json').is_file())


if __name__ == '__main__':
    unittest.main()
