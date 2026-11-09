"""Noncompartmental analysis of observed single-dose concentration-time profiles.

Observation times are times after the dose of the same subject and occasion.
Parameter keys follow CDISC SDTM PK parameter short names (PPTESTCD).
Steady-state, multiple-dose and infusion analyses are not supported.
"""

import math
import numbers

from . import _native
from ._analysis import fail

ROUTES = ("iv_bolus", "extravascular")
AUC_METHODS = ("linear", "linear_up_log_down")
BLQ_CHOICES = {
    "leading": ("zero", "error"),
    "embedded": ("error", "exclude", "zero"),
    "trailing": ("exclude", "error"),
}

# CDISC SDTM PK parameter short names and test names.
PARAMETERS = {
    "C0": "Initial Conc",
    "CMAX": "Max Conc",
    "TMAX": "Time of CMAX Observation",
    "CLST": "Last Nonzero Conc",
    "TLST": "Time of Last Nonzero Conc",
    "AUCLST": "AUC to Last Nonzero Conc",
    "LAMZ": "Lambda z",
    "LAMZHL": "Half-Life Lambda z",
    "LAMZNPT": "Number of Points for Lambda z",
    "LAMZLL": "Lambda z Lower Limit",
    "LAMZUL": "Lambda z Upper Limit",
    "LAMZSPN": "Lambda z Span",
    "R2": "R Squared",
    "R2ADJ": "R Squared Adjusted",
    "AUCIFO": "AUC Infinity Obs",
    "AUCIFP": "AUC Infinity Pred",
    "AUCPEO": "AUC %Extrapolation Obs",
    "AUCPEP": "AUC %Extrapolation Pred",
    "CLO": "Total CL Obs",
    "CLP": "Total CL Pred",
    "VZO": "Vz Obs",
    "VZP": "Vz Pred",
    "CLFO": "Total CL Obs by F",
    "CLFP": "Total CL Pred by F",
    "VZFO": "Vz Obs by F",
    "VZFP": "Vz Pred by F",
}


def _text(value, name):
    if not isinstance(value, str) or not value.strip():
        fail(f"{name} must be a nonempty string")
    return value


def _number(value, name):
    # numbers.Real covers NumPy scalars; NumPy booleans are not registered.
    if (
        isinstance(value, bool)
        or not isinstance(value, numbers.Real)
        or not math.isfinite(value)
    ):
        fail(f"{name} must be a finite number")
    return float(value)


def _integer(value, name):
    if isinstance(value, bool) or not isinstance(value, numbers.Integral):
        fail(f"{name} must be an integer")
    return int(value)


def _missing(value):
    # Data frames hold missing cells as None, NaN or pandas.NA.
    if value is None:
        return True
    if isinstance(value, numbers.Real) and not isinstance(value, bool):
        return math.isnan(value)
    return type(value).__name__ == "NAType"


def _flag(value, name):
    if _missing(value):
        return False
    if isinstance(value, bool):
        return value
    if getattr(getattr(value, "dtype", None), "kind", None) == "b":
        return bool(value)
    fail(f"{name} must be true or false")


def _key(row, label):
    key = []
    for name, value in (("subject", row.get("subject")), ("occasion", row.get("occasion", 1))):
        if isinstance(value, str):
            key.append(str(value))
        elif isinstance(value, numbers.Integral) and not isinstance(value, bool):
            key.append(int(value))
        else:
            fail(f"{label} {name} must be a string or integer")
    return tuple(key)


def _records(data, name):
    # pandas is optional; its public to_dict API avoids a package dependency.
    if hasattr(data, "columns") and hasattr(data, "to_dict"):
        data = data.to_dict(orient="records")
    if not isinstance(data, (list, tuple)) or not data:
        fail(f"{name} must be a nonempty sequence of row dicts or a data frame")
    if any(not isinstance(row, dict) for row in data):
        fail(f"every {name} row must be a dict")
    return data


def _blq_rule(blq):
    if blq is None:
        return None
    if not isinstance(blq, dict) or set(blq) != set(BLQ_CHOICES):
        fail("blq must be None or a dict with keys leading, embedded and trailing")
    for position, choices in BLQ_CHOICES.items():
        if blq[position] not in choices:
            fail(f"blq {position} must be one of: {', '.join(choices)}")
    return dict(blq)


def _lambda_z_rule(rule):
    if rule is None:
        rule = {"rule": "adjusted_r2"}
    if not isinstance(rule, dict) or rule.get("rule") not in (
        "adjusted_r2",
        "last_points",
    ):
        fail("lambda_z rule must be 'adjusted_r2' or 'last_points'")
    if rule["rule"] == "adjusted_r2":
        if set(rule) - {"rule", "min_points", "tolerance"}:
            fail("adjusted_r2 accepts min_points and tolerance")
        points = _integer(rule.get("min_points", 3), "lambda_z min_points")
        tolerance = rule.get("tolerance", 1e-4)
        if points < 3:
            fail("lambda_z min_points must be an integer of at least 3")
        tolerance = _number(tolerance, "lambda_z tolerance")
        if not 0 <= tolerance < 1:
            fail("lambda_z tolerance must be in [0, 1)")
        return {"rule": "adjusted_r2", "min_points": points, "tolerance": tolerance}
    if set(rule) != {"rule", "points"}:
        fail("last_points requires points")
    points = _integer(rule["points"], "lambda_z points")
    if points < 3:
        fail("lambda_z points must be an integer of at least 3")
    return {"rule": "last_points", "points": points}


def _units(units):
    if not isinstance(units, dict) or set(units) != {"time", "concentration", "amount"}:
        fail("units must declare time, concentration and amount")
    time, conc, amount = (
        _text(units[k], f"{k} unit") for k in ("time", "concentration", "amount")
    )
    auc = f"{conc}.{time}"  # UCUM product notation
    clearance, volume = f"{amount}/({auc})", f"{amount}/({conc})"
    try:
        factors = (
            _native.convert_interchange_quantity(1.0, clearance, f"L/{time}"),
            _native.convert_interchange_quantity(1.0, volume, "L"),
        )
        clearance, volume, derived = f"L/{time}", "L", "converted"
    except _native.NativeError:
        # Unrecognised or incompatible units (for example a mass dose with a
        # molar concentration) are reported literally, without conversion.
        factors, derived = (1.0, 1.0), "literal"
        if conc.startswith(amount + "/") and conc.count("/") == 1:
            per = conc[len(amount) + 1 :]
            clearance, volume = f"{per}/{time}", per
    return {
        "time": time,
        "concentration": conc,
        "amount": amount,
        "auc": auc,
        "rate": f"1/{time}",
        "clearance": clearance,
        "volume": volume,
        "derived": derived,
        "_factors": factors,
    }


def _segment(t1, c1, t2, c2, method):
    dt = t2 - t1
    if method == "linear_up_log_down" and c1 > c2 > 0:
        return (c1 - c2) / math.log(c1 / c2) * dt
    return (c1 + c2) / 2 * dt


def _regression(points):
    n = len(points)
    x = [t for t, _ in points]
    y = [math.log(c) for _, c in points]
    xm, ym = sum(x) / n, sum(y) / n
    sxx = sum((v - xm) ** 2 for v in x)
    sxy = sum((u - xm) * (v - ym) for u, v in zip(x, y))
    slope = sxy / sxx
    intercept = ym - slope * xm
    if slope >= 0:
        return None
    ss_res = sum((v - intercept - slope * u) ** 2 for u, v in zip(x, y))
    ss_tot = sum((v - ym) ** 2 for v in y)
    r2 = 1 - ss_res / ss_tot
    return {
        "lambda_z": -slope,
        "intercept": intercept,
        "r2": r2,
        "adjusted_r2": 1 - (1 - r2) * (n - 1) / (n - 2),
        "points": points,
    }


def _terminal(eligible, rule):
    if rule["rule"] == "last_points":
        needed = rule["points"]
        if len(eligible) < needed:
            return None, f"fewer than {needed} eligible terminal points"
        fit = _regression(eligible[-needed:])
        if fit is None:
            return None, "the selected terminal points do not decline"
        return fit, None
    needed = rule["min_points"]
    if len(eligible) < needed:
        return None, f"fewer than {needed} eligible terminal points"
    fits = [
        fit
        for n in range(needed, len(eligible) + 1)
        if (fit := _regression(eligible[-n:])) is not None
    ]
    if not fits:
        return None, "no candidate terminal set has a negative slope"
    best = max(fit["adjusted_r2"] for fit in fits)
    chosen = [fit for fit in fits if fit["adjusted_r2"] >= best - rule["tolerance"]]
    return max(chosen, key=lambda fit: len(fit["points"])), None


def _profile(key, rows, dose, method, blq, rule, units):
    label = f"subject {key[0]!r} occasion {key[1]!r}"
    rows = sorted(rows, key=lambda row: row["time"])
    for a, b in zip(rows, rows[1:]):
        if a["time"] == b["time"]:
            fail(f"{label} has duplicate observation time {a['time']}")
    record = []
    for row in rows:
        if row["time"] <= 0:
            if not row["blq"] and row["concentration"] != 0:
                fail(
                    f"{label} has a quantifiable pre-dose concentration "
                    f"{row['concentration']} at time {row['time']}; single-dose NCA "
                    "requires pre-dose rows that are BLQ or zero"
                )
            record.append({**row, "role": "predose"})
    post = [row for row in rows if row["time"] > 0]
    positive = [i for i, row in enumerate(post) if not row["blq"] and row["concentration"] > 0]
    if not positive:
        fail(f"{label} has no positive post-dose concentration")
    first, last = positive[0], positive[-1]
    used = []
    for i, row in enumerate(post):
        if not row["blq"]:
            role = "excluded_after_tlast" if i > last else "auc"
            record.append({**row, "role": role})
            if i <= last:
                used.append((row["time"], row["concentration"]))
            continue
        position = "leading" if i < first else "trailing" if i > last else "embedded"
        if blq is None:
            fail(
                f"{label} has a {position} BLQ row at time {row['time']}; "
                "pass blq={'leading': ..., 'embedded': ..., 'trailing': ...}"
            )
        action = blq[position]
        if action == "error":
            fail(f"{label} has a {position} BLQ row at time {row['time']}")
        if action == "zero":
            if dose["route"] == "iv_bolus" and position == "leading":
                fail(f"{label} has a BLQ row before the first IV bolus concentration")
            used.append((row["time"], 0.0))
            record.append({**row, "role": "blq_set_to_zero"})
        else:
            record.append({**row, "role": f"{position}_blq_excluded"})

    if dose["route"] == "iv_bolus":
        (t1, c1), (t2, c2) = used[0], used[1] if len(used) > 1 else (None, None)
        if c2 is None or not c1 > c2 > 0:
            fail(
                f"{label} needs two declining positive concentrations after an IV "
                "bolus to back-extrapolate C0"
            )
        c0 = c1 * (c1 / c2) ** (t1 / (t2 - t1))
        zero = {"concentration": c0, "source": "log_linear_back_extrapolation"}
    else:
        source = "predose_sample" if any(r["time"] == 0 for r in rows) else "imputed_zero"
        zero = {"concentration": 0.0, "source": source}
    curve = [(0.0, zero["concentration"]), *used]

    peak = max(range(len(used)), key=lambda i: (used[i][1], -i))
    tmax = used[peak][0]
    tlast, clast = used[-1]
    auclast = sum(
        _segment(t1, c1, t2, c2, method)
        for (t1, c1), (t2, c2) in zip(curve, curve[1:])
    )
    eligible = [
        (t, c)
        for t, c in used
        if c > 0 and (dose["route"] == "iv_bolus" or t > tmax)
    ]
    fit, reason = _terminal(eligible, rule)
    p = {
        "CMAX": used[peak][1],
        "TMAX": tmax,
        "CLST": clast,
        "TLST": tlast,
        "AUCLST": auclast,
    }
    if dose["route"] == "iv_bolus":
        p["C0"] = zero["concentration"]
    selected = {t for t, _ in fit["points"]} if fit else set()
    for row in record:
        if row["time"] in selected:
            row["role"] = "auc_and_lambda_z"
    terminal = {
        "status": "estimated" if fit else "not_estimated",
        "reason": reason,
        "eligible_points": len(eligible),
        "intercept": fit["intercept"] if fit else None,
        "predicted_clast": math.exp(fit["intercept"] - fit["lambda_z"] * tlast) if fit else None,
        "times": sorted(selected),
    }
    estimate = {}
    if fit:
        lz = fit["lambda_z"]
        times = [t for t, _ in fit["points"]]
        predicted = math.exp(fit["intercept"] - lz * tlast)
        half_life = math.log(2) / lz
        aucifo = auclast + clast / lz
        aucifp = auclast + predicted / lz
        estimate = {
            "LAMZ": lz,
            "LAMZHL": half_life,
            "LAMZNPT": len(times),
            "LAMZLL": times[0],
            "LAMZUL": times[-1],
            "LAMZSPN": (times[-1] - times[0]) / half_life,
            "R2": fit["r2"],
            "R2ADJ": fit["adjusted_r2"],
            "AUCIFO": aucifo,
            "AUCIFP": aucifp,
            "AUCPEO": 100 * (clast / lz) / aucifo,
            "AUCPEP": 100 * (predicted / lz) / aucifp,
        }
    suffix = "" if dose["route"] == "iv_bolus" else "F"
    amount = dose["amount"]
    to_clearance, to_volume = units["_factors"]
    for observed, area in (("O", "AUCIFO"), ("P", "AUCIFP")):
        auc = estimate.get(area)
        estimate[f"CL{suffix}{observed}"] = amount / auc * to_clearance if auc else None
        estimate[f"VZ{suffix}{observed}"] = (
            amount / (estimate["LAMZ"] * auc) * to_volume if auc else None
        )
    for code in (
        "LAMZ", "LAMZHL", "LAMZNPT", "LAMZLL", "LAMZUL", "LAMZSPN", "R2", "R2ADJ",
        "AUCIFO", "AUCIFP", "AUCPEO", "AUCPEP",
    ):
        p[code] = estimate.get(code)
    for code in (f"CL{suffix}O", f"CL{suffix}P", f"VZ{suffix}O", f"VZ{suffix}P"):
        p[code] = estimate[code]
    unit = {
        "C0": units["concentration"], "CMAX": units["concentration"],
        "CLST": units["concentration"],
        "TMAX": units["time"], "TLST": units["time"], "LAMZHL": units["time"],
        "LAMZLL": units["time"], "LAMZUL": units["time"],
        "AUCLST": units["auc"], "AUCIFO": units["auc"], "AUCIFP": units["auc"],
        "LAMZ": units["rate"], "AUCPEO": "%", "AUCPEP": "%",
        "LAMZNPT": "1", "LAMZSPN": "1", "R2": "1", "R2ADJ": "1",
    }
    parameters = {
        code: {
            "value": value,
            "unit": unit.get(code)
            or (units["clearance"] if code.startswith("CL") else units["volume"]),
            "name": PARAMETERS[code],
        }
        for code, value in p.items()
    }
    return {
        "subject": key[0],
        "occasion": key[1],
        "route": dose["route"],
        "dose": {"value": amount, "unit": units["amount"]},
        "time_zero": zero,
        "parameters": parameters,
        "lambda_z": terminal,
        "rows": [
            {k: row[k] for k in ("time", "concentration", "blq", "role")}
            for row in sorted(record, key=lambda row: row["time"])
        ],
    }


def noncompartmental_analysis(
    observations,
    doses,
    *,
    units,
    auc_method="linear_up_log_down",
    blq=None,
    lambda_z=None,
):
    """Estimate conventional single-dose NCA parameters for each subject and occasion.

    ``observations`` rows hold ``subject``, optional ``occasion`` (default 1),
    ``time`` after the dose, and either a finite ``concentration`` or
    ``blq=True`` with no concentration. ``doses`` rows hold ``subject``,
    optional ``occasion``, ``amount`` and ``route`` (``iv_bolus`` or
    ``extravascular``). ``units`` declares ``time``, ``concentration`` and
    ``amount`` units.

    Rows at or before time zero are pre-dose rows; they must be BLQ or zero.
    Post-dose BLQ rows raise an error unless ``blq`` assigns an action to
    leading, embedded and trailing positions. ``auc_method`` is ``linear`` or
    ``linear_up_log_down``. ``lambda_z`` selects the terminal points:
    ``{"rule": "adjusted_r2", "min_points": 3, "tolerance": 1e-4}`` (default)
    or ``{"rule": "last_points", "points": n}``. Parameters that need the
    terminal rate constant are ``None`` when it cannot be estimated, and the
    reason is reported. Extravascular clearance and volume are apparent values
    divided by an unknown bioavailability F.

    AUC is reported in the UCUM product unit ``concentration.time``. Clearance
    and volume are converted to ``L/time`` and ``L`` when the declared units
    are recognised and dimensionally compatible (``units["derived"]`` is
    ``"converted"``). Otherwise they are reported without conversion in the
    literal unit ``amount/(concentration.time)`` or ``amount/(concentration)``
    (``"literal"``). Missing cells (None, NaN) in a ``blq`` column mean not BLQ,
    and BLQ rows may hold a missing concentration.
    """
    if auc_method not in AUC_METHODS:
        fail("auc_method must be 'linear' or 'linear_up_log_down'")
    blq_rule = _blq_rule(blq)
    rule = _lambda_z_rule(lambda_z)
    declared = _units(units)
    dosing = {}
    for index, row in enumerate(_records(doses, "dose")):
        key = _key(row, f"dose row {index}")
        if key in dosing:
            fail(f"subject {key[0]!r} occasion {key[1]!r} has more than one dose")
        route = row.get("route")
        if route not in ROUTES:
            fail(f"dose row {index} route must be one of: {', '.join(ROUTES)}")
        amount = _number(row.get("amount"), f"dose row {index} amount")
        if amount <= 0:
            fail(f"dose row {index} amount must be positive")
        dosing[key] = {"route": route, "amount": amount}
    groups = {}
    for index, row in enumerate(_records(observations, "observation")):
        key = _key(row, f"observation row {index}")
        time = _number(row.get("time"), f"observation row {index} time")
        flagged = _flag(row.get("blq"), f"observation row {index} blq")
        value = row.get("concentration")
        if flagged:
            if not _missing(value):
                fail(f"observation row {index} is BLQ and must not carry a concentration")
            value = None
        else:
            value = _number(value, f"observation row {index} concentration")
            if value < 0:
                fail(f"observation row {index} concentration must not be negative")
        groups.setdefault(key, []).append(
            {"time": time, "concentration": value, "blq": flagged}
        )
    for key in groups:
        if key not in dosing:
            fail(f"subject {key[0]!r} occasion {key[1]!r} has no dose")
    for key in dosing:
        if key not in groups:
            fail(f"subject {key[0]!r} occasion {key[1]!r} has a dose but no observations")
    profiles = [
        _profile(key, rows, dosing[key], auc_method, blq_rule, rule, declared)
        for key, rows in groups.items()
    ]
    return {
        "schema": "qspquest.nca/v0.1",
        "method": {
            "auc": auc_method,
            "blq": blq_rule,
            "lambda_z": rule,
            "predose": "rows at or before time zero must be BLQ or zero",
        },
        "units": {k: v for k, v in declared.items() if not k.startswith("_")},
        "profiles": profiles,
        "table": [
            {
                "subject": profile["subject"],
                "occasion": profile["occasion"],
                "parameter": code,
                "value": item["value"],
                "unit": item["unit"],
            }
            for profile in profiles
            for code, item in profile["parameters"].items()
        ],
    }
