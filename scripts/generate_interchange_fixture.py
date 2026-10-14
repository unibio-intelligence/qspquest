"""Generate and check the independently authored synthetic OMEX example."""

import runpy
from pathlib import Path

root = Path(__file__).resolve().parents[1]
namespace = runpy.run_path(str(root / "docs/examples/interchange_roundtrip.py"))
print(namespace["run_tutorial"](root, root / "examples/synthetic"))
