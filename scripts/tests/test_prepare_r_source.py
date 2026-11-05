"""Admission checks for the offline R source-package build."""
import importlib.util
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "prepare-r-source.py"
spec = importlib.util.spec_from_file_location("r_source", SCRIPT)
r_source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r_source)


class LockedRSourceTests(unittest.TestCase):
    def test_required_legal_notices_are_complete_and_missing_credits_fail(self):
        text = ("Title\n" + "=" * 72 + "\n"
                "selected 1.0 — LICENSE\nunused 2.0 — LICENSE\n" + "-" * 72 + "\n"
                "Copyright (c) Example author\nPermission and conditions.\n" + "=" * 72 +
                "\nAdditional license conditions.\n")
        result = r_source.dependency_notices(text, [{"name": "selected", "version": "1.0"}])
        self.assertIn("Copyright (c) Example author\nPermission and conditions.", result)
        self.assertIn("Additional license conditions.", result)
        self.assertNotIn("unused 2.0", result)
        with self.assertRaisesRegex(ValueError, "Missing legal notices"):
            r_source.dependency_notices(text, [{"name": "missing", "version": "1.0"}])

    def test_pruning_unused_packages_preserves_locked_identities(self):
        with tempfile.TemporaryDirectory() as temp:
            original, selected = Path(temp) / "original", Path(temp) / "selected"
            dependency = ('[[package]]\nname = "synthetic"\nversion = "1.0.0"\n'
                          'source = "registry+https://example.invalid/index"\nchecksum = "abc"\n')
            original.write_text(dependency + '\n[[package]]\nname = "unused"\nversion = "2.0.0"\n')
            selected.write_text(dependency)
            r_source.validate_lock(original, selected)
            for before, after in (("1.0.0", "1.0.1"), ("abc", "def"),
                                  ("example.invalid", "other.invalid")):
                with self.subTest(change=before):
                    selected.write_text(dependency.replace(before, after))
                    with self.assertRaisesRegex(ValueError, "changed a locked dependency"):
                        r_source.validate_lock(original, selected)

    def test_copyright_holders_come_from_legal_files_without_templates(self):
        with tempfile.TemporaryDirectory() as temp:
            crate = Path(temp)
            (crate / "LICENSE-MIT").write_text(
                "Copyright (c) 2014 Alex Crichton <alex@example.invalid>\n"
                "Copyright 2019, 2022 The Example Developers. All rights reserved.\n")
            (crate / "LICENSE-APACHE").write_text(
                "Copyright [yyyy] [name of copyright owner]\n"
                "copyright license to reproduce, prepare Derivative Works of,\n"
                "Copyright (c) 2015\nCopyright (c) 2014 Alex Crichton\n")
            (crate / "README.md").write_text("Copyright 2020 Not A Legal File\n")
            self.assertEqual(r_source.copyright_holders(crate),
                             ["Alex Crichton", "The Example Developers"])

    def test_old_rust_is_rejected_before_unpacking_or_building(self):
        configure = SCRIPT.parents[1] / "bindings/r/configure"
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            tool = root / "old-rust"
            tool.write_text('#!/bin/sh\nprintf "rustc 1.70.0\\n"\n')
            tool.chmod(0o755)
            env = dict(os.environ, CARGO=str(tool), RUSTC=str(tool))
            result = subprocess.run(["sh", str(configure)], cwd=root, env=env,
                                    text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("requires Cargo and rustc >= 1.89", result.stderr)
            self.assertFalse((root / "src").exists())

    def test_suitable_personal_toolchain_is_found_after_old_system_tools(self):
        configure = SCRIPT.parents[1] / "bindings/r/configure"
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old, personal = root / "system", root / "home/.cargo/bin"
            old.mkdir()
            personal.mkdir(parents=True)
            for folder, version in ((old, "1.70.0"), (personal, "1.98.1")):
                for name in ("cargo", "rustc"):
                    tool = folder / name
                    tool.write_text('#!/bin/sh\nprintf "' + name + ' ' + version + '\\n"\n')
                    tool.chmod(0o755)
            env = dict(os.environ, HOME=str(root / "home"),
                       PATH=str(old) + os.pathsep + "/usr/bin:/bin", CARGO="", RUSTC="")
            result = subprocess.run(["sh", str(configure)], cwd=root, env=env,
                                    text=True, capture_output=True)
            self.assertIn("cargo 1.98.1", result.stdout)
            self.assertIn("rustc 1.98.1", result.stdout)
            self.assertIn("standalone R source archive", result.stderr)

    def test_cleanup_preserves_workspace_makevars(self):
        cleanup = SCRIPT.parents[1] / "bindings/r/cleanup"
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "src").mkdir()
            makevars = root / "src/Makevars"
            makevars.write_text("workspace build configuration\n")
            subprocess.run(["sh", str(cleanup)], cwd=root, check=True)
            self.assertEqual(makevars.read_text(), "workspace build configuration\n")
            (root / "src/rust-vendor.tar.xz").touch()
            subprocess.run(["sh", str(cleanup)], cwd=root, check=True)
            self.assertFalse(makevars.exists())


if __name__ == "__main__":
    unittest.main()
