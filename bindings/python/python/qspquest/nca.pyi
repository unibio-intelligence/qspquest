from collections.abc import Sequence
from typing import Any, Literal

def noncompartmental_analysis(
    observations: Sequence[dict[str, Any]] | Any,
    doses: Sequence[dict[str, Any]] | Any,
    *,
    units: dict[str, str],
    auc_method: Literal['linear', 'linear_up_log_down'] = ...,
    blq: dict[str, str] | None = ...,
    lambda_z: dict[str, Any] | None = ...,
) -> dict[str, Any]: ...
