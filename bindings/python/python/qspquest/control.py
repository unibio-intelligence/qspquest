"""Cooperative analysis control and verified, atomic JSON checkpoints."""

from pathlib import Path
import hashlib
import json
import os
import stat
import tempfile
import threading
from . import _native
from ._analysis import fail, integer


class AnalysisControl:
    """Progress callback, cooperative cancellation, and optional restart state.

    ``progress`` receives an independent dict after a complete work unit. Call
    ``cancel()`` from it or another thread. Native solves finish their bounded
    call before cancellation raises ``QSPQuestError(code='cancelled')``.
    Checkpoints contain JSON data written by the native analysis, never
    executable pickle. Reuse requires identical inputs and native build identity.
    A control is owned by one active analysis; create a new one to resume.
    """

    def __init__(
        self, *, progress=None, checkpoint=None, resume=False, checkpoint_every=10
    ):
        if progress is not None and not callable(progress):
            fail("progress must be callable")
        if resume and checkpoint is None:
            fail("resume requires a checkpoint path")
        self.progress = progress
        self.checkpoint = Path(checkpoint) if checkpoint is not None else None
        self.resume = bool(resume)
        self.checkpoint_every = integer(checkpoint_every, "checkpoint_every", 1, 1000)
        self._cancel = threading.Event()
        self._token = _native.CancelToken()
        self._context = None
        self.last_progress = None

    def cancel(self):
        """Request cancellation at the next native-call or work-unit boundary."""
        self._cancel.set()
        self._token.cancel()

    def check(self):
        if self._cancel.is_set():
            fail(
                "Analysis cancelled. Resume the last complete checkpoint with a new AnalysisControl; unfinished work is discarded.",
                "cancelled",
            )

    def start(self, operation, context):
        if self._context is not None:
            fail("an AnalysisControl may be used for only one analysis")
        context = json.loads(json.dumps(context))
        native = context.get("native")
        if isinstance(native, dict):
            # Per-run hashes vary with bound values; the build identity does not.
            for key in ("bound_value_hash", "run_request_hash"):
                native.pop(key, None)
        self._context = {"operation": operation, "context": context}
        self.check()
        if not self.resume:
            if self.checkpoint is not None and self.checkpoint.exists():
                fail(
                    "checkpoint already exists; use resume=True or choose a new path",
                    "checkpoint",
                )
            return None
        path = self.checkpoint
        try:
            fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
            try:
                info = os.fstat(fd)
                if not stat.S_ISREG(info.st_mode) or info.st_size > 200000000:
                    fail(
                        "checkpoint must be a regular JSON file below 200 MB",
                        "checkpoint",
                    )
                with os.fdopen(fd, "r", encoding="utf-8", closefd=False) as source:
                    envelope = json.load(source)
            finally:
                os.close(fd)
        except (OSError, UnicodeError, json.JSONDecodeError) as exception:
            fail(
                "checkpoint could not be read as regular UTF-8 JSON: " + str(exception),
                "checkpoint",
            )
        if not isinstance(envelope, dict) or not isinstance(
            envelope.get("payload"), dict
        ):
            fail("checkpoint envelope is invalid", "checkpoint")
        payload = envelope.get("payload")
        if envelope.get("schema") == "qspquest.checkpoint/v0.1":
            fail(
                "checkpoint was written by an earlier analysis version with a different random stream; start a new analysis",
                "checkpoint",
            )
        if envelope.get("schema") != "qspquest.checkpoint/v0.2" or envelope.get(
            "sha256"
        ) != _digest(payload):
            fail("checkpoint integrity check failed", "checkpoint")
        if payload.get("context") != self._context:
            fail(
                "checkpoint inputs, implementation, native build or environment differ; start a new analysis",
                "checkpoint",
            )
        if not isinstance(payload.get("state"), dict):
            fail("checkpoint state is invalid", "checkpoint")
        return payload["state"]

    def _native_start(self, operation, context):
        state = self.start(operation, json.loads(context))
        return None if state is None else json.dumps(state, allow_nan=False)

    def _wants_state(self, completed, final):
        return self.checkpoint is not None and (
            final or completed % self.checkpoint_every == 0
        )

    def _native_boundary(self, progress, state):
        event = json.loads(progress)
        event["checkpoint"] = str(self.checkpoint) if self.checkpoint else None
        self.last_progress = event.copy()
        if state is not None:
            self._save({"context": self._context, "state": json.loads(state)})
        if self.progress is not None:
            self.progress(event.copy())

    def _native_retain(self, state):
        self._retain_checkpoint(json.loads(state))

    def _retain_checkpoint(self, state):
        """Keep completed state and final charged work after drained cancellation."""
        if self.checkpoint is not None and self._context is not None:
            self._save({"context": self._context, "state": state})

    def _save(self, payload):
        document = json.dumps(
            {
                "schema": "qspquest.checkpoint/v0.2",
                "sha256": _digest(payload),
                "payload": payload,
            },
            allow_nan=False,
            separators=(",", ":"),
        )
        path = self.checkpoint
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_symlink() or (path.exists() and not path.is_file()):
            fail("checkpoint destination must be a regular file", "checkpoint")
        fd, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as target:
                target.write(document)
                target.flush()
                os.fsync(target.fileno())
            os.replace(temporary, path)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)


def _digest(value):
    return hashlib.sha256(
        json.dumps(
            value, sort_keys=True, separators=(",", ":"), allow_nan=False
        ).encode()
    ).hexdigest()
