"""Compile once and run versioned pharmacology requests with the native engine."""

import json
from . import _native

__version__ = _native.__version__
__all__ = [
    "CompiledSensitivities",
    "CompiledModel",
    "QSPQuestError",
    "PopulationFitDataset",
    "population_fit_dataset",
    "format_model",
    "fit_gamma_poisson",
    "fit_probit_normal_binomial",
    "fitting_identity",
    "__version__",
    "fit_data",
    "fit_population",
    "fit_diagnostics",
    "visual_predictive_check",
    "profile_likelihood",
    "bootstrap_population",
    "exposure_summary",
    "compare_regimens",
    "event_sensitivities",
    "write_analysis_report",
]


class QSPQuestError(ValueError):
    """A structured compiler or scientific execution failure with no partial result."""

    def __init__(self, diagnostic):
        self.diagnostic = diagnostic
        self.code = diagnostic.get("code", "internal")
        self.time = diagnostic.get("time")
        self.expression = diagnostic.get("expression")
        self.line = diagnostic.get("line")
        self.column = diagnostic.get("column")
        super().__init__(diagnostic.get("message", self.code))

    def explain(self):
        """Return a plain explanation and interpretation boundary."""
        return scientific_status(self)


def _encode(value):
    if isinstance(value, str):
        return value
    if not isinstance(value, dict):
        raise TypeError("expected model text, JSON text, or a dict")
    try:
        return json.dumps(value, allow_nan=False)
    except (ValueError, TypeError) as error:
        raise QSPQuestError(
            {"code": "invalid_input", "message": str(error)}
        ) from error


def _call(function, *args):
    try:
        return function(*args)
    except _native.NativeError as error:
        raise QSPQuestError(json.loads(str(error))) from None


class CompiledModel:
    """Immutable compiled model; independent request bindings on each run.

    Compilation and native execution release the GIL. A compiled model can be
    shared across Python threads; returned dicts and numeric lists are independent.
    """

    __slots__ = ("_model",)

    def __init__(self, source):
        self._model = _call(_native.CompiledModel, _encode(source))

    @property
    def model_content_hash(self):
        return self._model.model_content_hash

    @property
    def output_names(self):
        return self._model.output_names

    @property
    def output_units(self):
        return self._model.output_units

    def morris_json(self, request):
        return _call(self._model.morris_json, _encode(request))

    def morris(self, request):
        """Execute explicit or seeded Morris screening with bounded scans."""
        return json.loads(self.morris_json(request))

    def scan_json(self, request):
        return _call(self._model.scan_json, _encode(request))

    def scan(self, request):
        """Run up to 25 explicit parameter points under aggregate budgets."""
        return json.loads(self.scan_json(request))

    def validate_run(self, request):
        """Admit a fixed-regimen interchange request without integrating.

        Supports diffsol_bdf, parameters and fixed bolus/infusion regimens.
        Model-level lag/history/switches, covariates and per-state tolerances
        require the execution API and are outside this admission contract.
        """
        return json.loads(_call(self._model.validate_run_json, _encode(request)))

    def run(self, request):
        """Return a qspquest.result/v0.1 dict, including execution identity."""
        return AnalysisResult(json.loads(self.run_json(request)))

    def periodic_steady_state(self, request):
        """Return periodic pre-event amounts, residuals and execution identity."""
        return json.loads(
            _call(self._model.periodic_steady_state_json, _encode(request))
        )

    def iterate_periodic_state(self, request):
        """Iterate a phase-periodic model; return verified state and cycle count."""
        return json.loads(
            _call(self._model.iterate_periodic_state_json, _encode(request))
        )

    def run_json(self, request):
        """Return the complete result as JSON for storage or interprocess transfer."""
        return _call(self._model.execute_json, _encode(request))

    def fit_scalar_json(self, request):
        """Fit one bounded parameter through native simulations without sensitivities."""
        return _call(self._model.fit_scalar_json, _encode(request))

    def fit_scalar(self, request):
        return json.loads(self.fit_scalar_json(request))

    def fit_data(self, run, observations, parameters, error, **options):
        """Bounded likelihood fitting, including request-time event controls."""
        return AnalysisResult(
            fit_data(self, run, observations, parameters, error, **options)
        )

    def fit_population(self, subjects, parameters, omega, error, **options):
        """General SAEM over native simulations; see its explicit method scope."""
        return AnalysisResult(
            fit_population(self, subjects, parameters, omega, error, **options)
        )

    def visual_predictive_check(self, subjects, parameters, omega, error, **options):
        return AnalysisResult(
            visual_predictive_check(self, subjects, parameters, omega, error, **options)
        )

    def profile_likelihood(
        self, run, observations, parameters, error, parameter, grid, **options
    ):
        return AnalysisResult(
            profile_likelihood(
                self, run, observations, parameters, error, parameter, grid, **options
            )
        )

    def bootstrap_population(self, subjects, parameters, omega, error, **options):
        return AnalysisResult(
            bootstrap_population(self, subjects, parameters, omega, error, **options)
        )

    def compare_regimens(self, regimens, parameters, omega, output, **options):
        return AnalysisResult(
            compare_regimens(self, regimens, parameters, omega, output, **options)
        )

    def event_sensitivities(self, run, parameters, **options):
        return AnalysisResult(event_sensitivities(self, run, parameters, **options))


def fitting_identity(source, request, operation, *, result=None):
    """Normalize scalar/individual JSON and return native JCS identities.

    This checks request/model shapes, including default fields. It does not
    compile, admit a simulation, fit data, or establish scientific qualification.
    Source must be model JSON; operation is fit_scalar or fit_individual.
    Optional result is parsed through the matching native result type, without
    asserting convergence, recovery, or scientific qualification.
    """
    return json.loads(_call(_native.fitting_identity, _encode(source), _encode(request), operation,
                            None if result is None else _encode(result)))


def format_model(source):
    """Return canonical text after validating current engine support."""
    return _call(_native.format_model, _encode(source))


def fit_gamma_poisson(request):
    """Fit independent count/exposure rows with a shared Gamma shape and rate.

    The native bounded likelihood integrates the individual Poisson rates.
    Exposure quantities must be compatible with the Gamma rate parameter unit.
    """
    return json.loads(_call(_native.fit_gamma_poisson_json, _encode(request)))


def fit_probit_normal_binomial(request):
    """Fit two groups of independent binomial litters with normal probit effects.

    Means and SDs are dimensionless. Native optimization uses variance
    coordinates and includes the full unbounded normal mixture.
    """
    return json.loads(_call(_native.fit_probit_normal_binomial_json, _encode(request)))


class CompiledSensitivities:
    """Compile one ordered parameter selection; each run binds independent values.

    Native compilation/execution release the GIL. Instances can be shared across
    threads; the request's with_respect_to must match the compiled selection.
    Sensitivity requests accept diffsol_bdf (default in run helpers) or explicit
    diffsol_rodas5p, also used throughout individual and population fitting.
    """

    __slots__ = ("_model",)

    def __init__(self, source, with_respect_to):
        self._model = _call(
            _native.CompiledSensitivities, _encode(source), with_respect_to
        )

    def run_json(self, request):
        return _call(self._model.execute_json, _encode(request))

    def run(self, request):
        return AnalysisResult(json.loads(self.run_json(request)))

    def fit_json(self, request):
        """Return a versioned fit result; inspect fit.status for convergence."""
        return _call(self._model.fit_json, _encode(request))

    def plan_fit(self, request):
        """Validate a native fit and return its identity/semantics without solving.

        Observation rows, output dimensions, residual errors, parameter bounds
        and limits are checked by the same native binder used during execution.
        Admission is not convergence or statistical qualification.
        """
        return json.loads(_call(self._model.plan_fit_json, _encode(request)))

    def objective(self, request):
        """Evaluate a native GaussianObjectiveRequest and its ordered gradient.

        Rows may name different endpoints with explicit endpoint-specific error
        quantities. This executes sensitivities; it does not fit parameters.
        """
        return json.loads(_call(self._model.objective_json, _encode(request)))

    def fit(self, request):
        """Fit an individual, pooled, or population request in native code."""
        return json.loads(self.fit_json(request))

    def fit_population_data(self, dataset):
        """Fit a prepared FOCEI/SAEM dataset and retain source-row provenance."""
        if not isinstance(dataset, PopulationFitDataset):
            raise TypeError("dataset must come from population_fit_dataset")
        result = self.fit(dataset.request)
        population = result.get("population")
        if not isinstance(population, dict):
            raise QSPQuestError(
                {
                    "code": "internal",
                    "message": "population fit returned no population result",
                }
            )
        rows = population.get("fitted_observations")
        mapped = None
        if rows is not None:
            by_row = {
                (item["subject_index"], item["observation_index"]): item
                for item in dataset.mapping
            }
            if len(rows) != len(by_row):
                raise QSPQuestError(
                    {
                        "code": "internal",
                        "message": "fitted observation count differs from source mapping",
                    }
                )
            mapped = []
            seen = set()
            for row in rows:
                key = (row["subject_index"], row["observation_index"])
                source = by_row.get(key)
                if key in seen or source is None or row["row"] != source["result_row"]:
                    raise QSPQuestError(
                        {
                            "code": "internal",
                            "message": "fitted observation row differs from source mapping",
                        }
                    )
                seen.add(key)
                mapped.append(
                    {
                        **row,
                        "subject_id": source["subject_id"],
                        "source_row": source["source_row"],
                    }
                )
        return {
            "result": result,
            "subject_ids": list(dataset.subject_ids),
            "mapping": [item.copy() for item in dataset.mapping],
            "fitted_observations": mapped,
        }


from .dataset import PopulationFitDataset, population_fit_dataset
from .individual import IndividualFitDataset, individual_fit_dataset
__all__ += ["IndividualFitDataset", "individual_fit_dataset"]
from .estimation import fit_data
from .population import fit_population
from .diagnostics import fit_diagnostics, visual_predictive_check
from .uncertainty import profile_likelihood, bootstrap_population
from .exposure import exposure_summary, compare_regimens
from .derivatives import event_sensitivities
from .report import write_analysis_report

from .control import AnalysisControl
from .friendly import (
    quantity,
    bolus,
    infusion,
    run_request,
    AnalysisResult,
    scientific_status,
    analysis_subjects,
    fit_population_dataset,
)

__all__ += [
    "AnalysisControl",
    "quantity",
    "bolus",
    "infusion",
    "run_request",
    "AnalysisResult",
    "scientific_status",
    "analysis_subjects",
    "fit_population_dataset",
]

from .types import (
    Quantity, RunRequest, RunResult, SensitivityRequest, SensitivityResult,
    FitRequest, FitResult,
)
from .analysis_types import (
    ParameterBounds, ParameterSpec, ResidualError, FixedResidualError,
    AnalysisObservation, AnalysisSubject, CovariateSpec, ProgressEvent,
)

__all__ += [
    "Quantity", "RunRequest", "RunResult", "SensitivityRequest", "SensitivityResult",
    "FitRequest", "FitResult", "ParameterBounds", "ParameterSpec", "ResidualError",
    "FixedResidualError", "AnalysisObservation", "AnalysisSubject", "CovariateSpec",
    "ProgressEvent",
]


def model_library(root=None):
    """Open the shared model catalog; Docker mounts it at QSPQUEST_LIBRARY_DIR.

    The model-library loader must be installed or available on PYTHONPATH.
    Archives and indexed members are verified before a named scenario runs.
    """
    import os
    from pathlib import Path

    root = root or os.environ.get("QSPQUEST_LIBRARY_DIR")
    if not root or not (Path(root) / "catalog.json").is_file():
        raise ValueError(
            "Set QSPQUEST_LIBRARY_DIR to a model-library directory containing catalog.json, "
            "or pass that directory to model_library(root)."
        )
    try:
        from qspquest_models import Library
    except ImportError as error:
        raise ImportError(
            "The model-library loader is unavailable. In Docker, mount the complete "
            "library at /models and set PYTHONPATH=/models. For native Python, "
            "install the model-library package into this environment."
        ) from error
    return Library(root)


__all__ += ["model_library"]
