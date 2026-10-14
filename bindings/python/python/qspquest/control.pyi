from collections.abc import Callable
from os import PathLike
from pathlib import Path
from typing import Any
from .analysis_types import ProgressEvent

class AnalysisControl:
    progress: Callable[[ProgressEvent], None] | None
    checkpoint: Path | None
    resume: bool
    checkpoint_every: int
    last_progress: ProgressEvent | None
    def __init__(self, *, progress: Callable[[ProgressEvent], None] | None = ..., checkpoint: str | PathLike[str] | None = ..., resume: bool = ..., checkpoint_every: int = ...) -> None: ...
    def cancel(self) -> None: ...
    def check(self) -> None: ...
    def start(self, operation: str, identity: str, native: dict[str, Any] | None = ...) -> dict[str, Any] | None: ...
    def boundary(self, completed: int, total: int, budget: Any, state: dict[str, Any], *, final: bool = ...) -> None: ...
