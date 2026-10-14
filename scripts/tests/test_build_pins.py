"""Regressions for rejecting mutable release build inputs."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_build_pins import check


class BuildPins(unittest.TestCase):
    def test_mutable_actions_and_images_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workflow = root / ".github/workflows/ci.yml"
            workflow.parent.mkdir(parents=True)
            workflow.write_text("steps:\n  - uses: actions/checkout@v4\n")
            dockerfile = root / "Dockerfile"
            dockerfile.write_text("# syntax=docker/dockerfile:1\nARG BASE=debian:bookworm-slim\nFROM ${BASE}\n")
            self.assertEqual(len(check(root)), 3)
            workflow.write_text("steps:\n  - uses: 'actions/checkout@" + "a" * 40 + "' # v4\n")
            dockerfile.write_text("# syntax=docker/dockerfile:1@sha256:" + "b" * 64 + "\n"
                                  "ARG BASE=debian:bookworm-slim@sha256:" + "c" * 64 + "\n"
                                  "FROM ${BASE} AS runtime\nFROM runtime AS final\n")
            self.assertEqual(check(root), [])
            workflow.write_text("jobs:\n  ci:\n    uses: actions/checkout@" + "a" * 39 + " # v4\n")
            self.assertEqual(len(check(root)), 1)


if __name__ == "__main__":
    unittest.main()
