"""Bounded model and event-table adapters for common PK/PD software.

No source is evaluated. Import supports deterministic individual ODE models;
unsupported syntax raises AdapterError. See docs/SOFTWARE-ADAPTERS.md.
"""

from ._core import AdapterError, AdaptedModel, SOFTWARE, clean_source, native_model
from ._data import SimulationDataset, export_dataset, simulation_dataset
from ._export import ExportedModel, export_model
from ._readers import READERS
from ._bridge import import_rxode2_snapshot

__all__ = [
    "AdapterError",
    "AdaptedModel",
    "ExportedModel",
    "SimulationDataset",
    "import_model",
    "export_model",
    "simulation_dataset",
    "export_dataset",
    "import_rxode2_snapshot",
    "SOFTWARE",
]


def import_model(
    source,
    software,
    *,
    model_id="imported_model",
    time_unit,
    parameters=None,
    outputs=None,
    dosing_targets=None,
    structural_only=False,
):
    """Translate the supported source subset and compile it with the native engine.

    parameters supplies explicit scalar source parameter/covariate/individual-
    effect values. outputs optionally maps readout labels to source expressions.
    structural_only=True retains statistical declarations as unexecuted metadata;
    it never creates a population fit or samples random effects.
    """
    text = clean_source(source, software)
    try:
        program = READERS[software](text, structural_only=structural_only)
        return native_model(
            program,
            software,
            source,
            model_id,
            time_unit,
            parameters,
            outputs,
            dosing_targets,
        )
    except AdapterError as error:
        error.software = software
        raise
