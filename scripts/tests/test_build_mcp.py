"""Check feature-specific build orchestration without compiling or downloading."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MOCK = '''#!/usr/bin/env python3
import json, os
from pathlib import Path
import sys
name=Path(sys.argv[0]).name
with Path(os.environ['QSPQUEST_BUILD_TRACE']).open('a') as stream:
    stream.write(json.dumps([name,*sys.argv[1:]])+'\\n')
if name=='wasm-bindgen' and '--version' in sys.argv:
    print('wasm-bindgen '+os.environ.get('QSPQUEST_TEST_BINDGEN_VERSION','0.2.129'))
elif name=='cargo' and 'qspquest-wasm' in sys.argv:
    target=Path('target/wasm32-unknown-unknown/release/qspquest_wasm.wasm')
    target.parent.mkdir(parents=True,exist_ok=True)
    target.write_bytes(b'sandbox-fixture' if 'sandbox-abi' in sys.argv else b'browser-fixture')
elif name=='wasm-bindgen':
    target=Path(sys.argv[sys.argv.index('--out-dir')+1])
    target.mkdir(parents=True,exist_ok=True)
    (target/'qspquest_wasm.js').write_text('synthetic orchestration fixture')
'''


class BuildMCPTests(unittest.TestCase):
    def invoke(self, features, *, version='0.2.129', check=False):
        with tempfile.TemporaryDirectory(prefix='qspquest-build-orchestration-') as raw:
            root = Path(raw)
            binary = root / 'tools'
            binary.mkdir()
            for name in ('cargo', 'rustup', 'wasm-bindgen'):
                tool = binary / name
                tool.write_text(MOCK)
                tool.chmod(0o700)
            script = root / 'build-mcp.sh'
            shutil.copyfile(ROOT / 'scripts/build-mcp.sh', script)
            trace = root / 'commands.jsonl'
            env = dict(os.environ, PATH=str(binary) + os.pathsep + os.environ['PATH'],
                       QSPQUEST_BUILD_TRACE=str(trace), QSPQUEST_TEST_BINDGEN_VERSION=version)
            result = subprocess.run(['sh', str(script), '--check-features' if check else '--features', features],
                                    cwd=root, env=env, capture_output=True, text=True, timeout=10)
            calls = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
            files = {str(path.relative_to(root)): path.read_bytes()
                     for path in (root / 'target').rglob('*') if path.is_file()}
            return result, calls, files

    def test_native_features_never_invoke_wasm_tools(self):
        for features in ('', 'analysis', 'managed', 'analysis,managed'):
            with self.subTest(features=features):
                result, calls, files = self.invoke(features)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, [['cargo', 'build', '-p', 'qspquest-mcp', '--release', '--locked', '--features', features]])
                self.assertFalse(files)

    def test_sandbox_and_widget_build_only_their_required_assets(self):
        for features in ('sandbox', 'widget', 'sandbox,widget'):
            with self.subTest(features=features):
                result, calls, files = self.invoke(features)
                self.assertEqual(result.returncode, 0, result.stderr)
                builds = [call for call in calls if call[0] == 'cargo' and 'qspquest-wasm' in call]
                self.assertEqual(len(builds), len(features.split(',')))
                self.assertEqual(any('sandbox-abi' in call for call in builds), 'sandbox' in features)
                self.assertEqual(any(call[0] == 'wasm-bindgen' and '--target' in call for call in calls), 'widget' in features)
                if 'sandbox' in features:
                    self.assertEqual(files['target/mcp-assets/sandbox.wasm'], b'sandbox-fixture')
                else:
                    self.assertNotIn('target/mcp-assets/sandbox.wasm', files)
                self.assertEqual(calls[-1][-2:], ['--features', features])

    def test_invalid_features_and_incompatible_widget_tools_stop_before_building(self):
        for features in ('sandbox,unknown', 'sandbox,,widget', 'Sandbox', 'sandbox;echo', ',widget', 'widget,'):
            result, calls, _ = self.invoke(features)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(calls)
        result, calls, _ = self.invoke('widget', version='0.2.128')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [['wasm-bindgen', '--version']])

    def test_validation_mode_performs_no_tool_calls(self):
        result, calls, _ = self.invoke('sandbox,widget,analysis,managed', check=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(calls)


if __name__ == '__main__':
    unittest.main()
