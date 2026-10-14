"""Import an explicit JSON snapshot produced by the optional R converter bridge."""

from copy import deepcopy
from hashlib import sha256
import json

from ._core import fail, MAX_SOURCE_BYTES


def import_rxode2_snapshot(
    document,
    *,
    time_unit,
    parameters=None,
    outputs=None,
    dosing_targets=None,
    structural_only=False,
    model_id="rxode2_import",
):
    """Read a snapshot of an rxode2 UI, nonmem2rx UI or monolix2rx UI.

    This function never starts R, installs packages, reads vendor project paths,
    or evaluates the original source. The optional R helper creates the snapshot
    in a separate user-controlled R session.
    """
    from . import import_model

    if isinstance(document, str):
        if len(document.encode()) > MAX_SOURCE_BYTES:
            fail("Snapshot exceeds 1 MB", code="resource_limit")
        try:
            document = json.loads(document)
        except (ValueError, RecursionError):
            fail("Invalid snapshot JSON", code="invalid_document")
    allowed = {
        "schema",
        "model_source",
        "parameters",
        "states",
        "source_function",
        "versions",
    }
    if (
        not isinstance(document, dict)
        or set(document) != allowed
        or document["schema"] != "qspquest.rxode2-ui/v1"
    ):
        fail("Unsupported rxode2 snapshot schema", code="invalid_document")
    bindings = document["parameters"]
    if bindings == []:
        bindings = {}
    if not isinstance(bindings, dict) or not isinstance(document["states"], list):
        fail("Invalid snapshot bindings/states", code="invalid_document")
    bindings = {**bindings, **dict(parameters or {})}
    result = import_model(
        document["model_source"],
        "rxode2",
        time_unit=time_unit,
        parameters=bindings,
        outputs=outputs,
        dosing_targets=dosing_targets,
        structural_only=structural_only,
        model_id=model_id,
    )
    if list(result._program.states) != document["states"]:
        fail("Snapshot state order differs from source equations", code="integrity")
    if not isinstance(document["source_function"], str) or not isinstance(
        document["versions"], dict
    ):
        fail("Invalid snapshot provenance", code="invalid_document")
    result.source_sha256 = sha256(
        json.dumps(document, sort_keys=True, allow_nan=False).encode()
    ).hexdigest()
    result.source_metadata += (
        deepcopy(
            {
                "bridge": "qspquest.rxode2-ui/v1",
                "source_function": document["source_function"],
                "versions": document["versions"],
            }
        ),
    )
    return result
