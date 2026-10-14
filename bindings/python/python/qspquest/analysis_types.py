"""Dictionary contracts for Python analysis inputs and cooperative progress."""
from __future__ import annotations
from typing import Literal, TypedDict

from .types import Quantity, RunRequest


class ParameterBounds(TypedDict):
    initial: float
    lower: float
    upper: float


class _ParameterSpecRequired(ParameterBounds):
    unit: str


class ParameterSpec(_ParameterSpecRequired, total=False):
    path: list[str | int]


class ResidualError(TypedDict, total=False):
    model: Literal['normal', 'lognormal']
    additive: float | ParameterBounds
    proportional: float | ParameterBounds


class FixedResidualError(TypedDict, total=False):
    model: Literal['normal', 'lognormal']
    additive: float
    proportional: float


class _AnalysisObservationRequired(TypedDict):
    row: int
    output: str
    value: Quantity


class AnalysisObservation(_AnalysisObservationRequired, total=False):
    censoring: Literal['left'] | None
    source_row: int
    lloq: Quantity


class _AnalysisSubjectRequired(TypedDict):
    run: RunRequest
    observations: list[AnalysisObservation]


class AnalysisSubject(_AnalysisSubjectRequired, total=False):
    id: str | int
    covariates: dict[str, float | str]


class CovariateSpec(ParameterBounds):
    parameter: str
    covariate: str
    reference: float


class CovariateEstimate(TypedDict):
    parameter: str
    covariate: str
    reference: float
    estimate: float


class ProgressEvent(TypedDict):
    operation: str
    completed: int
    total: int
    simulations: int
    max_simulations: int
    checkpoint: str | None


class ScientificStatus(TypedDict):
    status: str
    title: str
    message: str
    interpretation: str
    warnings: list[str]
