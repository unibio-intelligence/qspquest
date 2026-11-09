"""Closed-form and edge-case checks for noncompartmental analysis."""

import math

import pytest

import qspquest as qq

UNITS = {"time": "h", "concentration": "mg/L", "amount": "mg"}
DOSE, V, CL, KA, F = 300.0, 60.0, 2.4, 0.9, 0.7
K = CL / V
LEADING_ZERO = {"leading": "zero", "embedded": "error", "trailing": "exclude"}


def bolus(t, dose=DOSE, v=V, k=K):
    return dose / v * math.exp(-k * t)


def oral(t, ka=KA, k=K, f=F, dose=DOSE, v=V):
    return f * dose * ka / (v * (ka - k)) * (math.exp(-k * t) - math.exp(-ka * t))


def rows(function, times, subject=1, predose=True):
    out = [{"subject": subject, "time": 0.0, "blq": True}] if predose else []
    return out + [
        {"subject": subject, "time": t, "concentration": function(t)} for t in times
    ]


def run(observations, route="extravascular", **options):
    return qq.noncompartmental_analysis(
        observations,
        [{"subject": 1, "amount": DOSE, "route": route}],
        units=UNITS,
        **options,
    )["profiles"][0]


def value(profile, code):
    return profile["parameters"][code]["value"]


def test_iv_bolus_monoexponential_recovers_closed_form_exactly():
    times = [0.5, 1, 2, 4, 8, 12, 24, 36, 48]
    profile = run(rows(bolus, times, predose=False), route="iv_bolus")
    # Log back-extrapolation and log-down trapezoids are exact for one exponential.
    assert value(profile, "C0") == pytest.approx(DOSE / V, rel=1e-12)
    assert value(profile, "LAMZ") == pytest.approx(K, rel=1e-12)
    assert value(profile, "LAMZHL") == pytest.approx(math.log(2) / K, rel=1e-12)
    assert value(profile, "LAMZNPT") == len(times)
    assert value(profile, "R2ADJ") == pytest.approx(1.0, abs=1e-12)
    assert value(profile, "AUCLST") == pytest.approx(
        DOSE / CL * (1 - math.exp(-K * 48)), rel=1e-12
    )
    for code in ("AUCIFO", "AUCIFP"):
        assert value(profile, code) == pytest.approx(DOSE / CL, rel=1e-12)
    assert value(profile, "CLO") == pytest.approx(CL, rel=1e-12)
    assert value(profile, "VZO") == pytest.approx(V, rel=1e-12)
    assert profile["parameters"]["CLO"]["unit"] == "L/h"
    assert profile["parameters"]["VZO"]["unit"] == "L"
    assert "CLFO" not in profile["parameters"]
    linear = run(
        rows(bolus, times, predose=False), route="iv_bolus", auc_method="linear"
    )
    # The linear rule overestimates the area of a convex declining curve.
    assert value(linear, "AUCLST") > value(profile, "AUCLST")


def test_oral_terminal_slope_equals_elimination_rate_constant():
    times = [0.5, 1, 2, 3, 24, 36, 48, 72, 96]
    profile = run(rows(oral, times))
    assert profile["time_zero"] == {"concentration": 0.0, "source": "predose_sample"}
    assert value(profile, "TMAX") == 3
    assert value(profile, "LAMZLL") == 24
    assert value(profile, "LAMZUL") == 96
    assert value(profile, "LAMZ") == pytest.approx(K, rel=1e-8)
    assert value(profile, "LAMZSPN") == pytest.approx(72 * K / math.log(2), rel=1e-8)
    assert profile["lambda_z"]["predicted_clast"] == pytest.approx(oral(96), rel=1e-6)
    assert "CLO" not in profile["parameters"]
    dense = run(
        rows(oral, [i / 10 for i in range(1, 10 * 144 + 1)]),
        lambda_z={"rule": "adjusted_r2", "tolerance": 0.0},
    )
    assert value(dense, "LAMZ") == pytest.approx(K, rel=1e-8)
    # The remaining difference is trapezoidal discretization error in AUClast.
    assert value(dense, "AUCIFO") == pytest.approx(F * DOSE / CL, rel=1e-4)
    assert value(dense, "CLFO") == pytest.approx(CL / F, rel=1e-4)
    assert value(dense, "VZFO") == pytest.approx(V / F, rel=1e-4)


def test_flip_flop_terminal_slope_equals_absorption_rate_constant():
    ka, k = 0.05, 0.5

    def flip(t):
        return oral(t, ka=ka, k=k, f=1.0)

    profile = run(rows(flip, [0.5, 1, 2, 4, 6, 60, 72, 96, 120]))
    assert value(profile, "LAMZLL") == 60
    assert value(profile, "LAMZ") == pytest.approx(ka, rel=1e-8)
    # NCA reports the apparent terminal half-life, here the absorption half-life.
    assert value(profile, "LAMZHL") == pytest.approx(math.log(2) / ka, rel=1e-8)


def test_linear_up_log_down_rule_uses_log_only_for_declining_segments():
    observations = [
        {"subject": 1, "time": 1.0, "concentration": 2.0},
        {"subject": 1, "time": 2.0, "concentration": 4.0},
        {"subject": 1, "time": 3.0, "concentration": 4.0},
        {"subject": 1, "time": 4.0, "concentration": 1.0},
    ]
    profile = run(observations)
    assert profile["time_zero"]["source"] == "imputed_zero"
    expected = 1.0 + 3.0 + 4.0 + 3.0 / math.log(4.0)
    assert value(profile, "AUCLST") == pytest.approx(expected, rel=1e-14)
    linear = run(observations, auc_method="linear")
    assert value(linear, "AUCLST") == pytest.approx(1.0 + 3.0 + 4.0 + 2.5)


def test_too_few_terminal_points_reports_reason_without_terminal_parameters():
    profile = run(rows(oral, [1, 2, 3, 24, 48]))
    assert profile["lambda_z"]["status"] == "not_estimated"
    assert profile["lambda_z"]["reason"] == "fewer than 3 eligible terminal points"
    assert value(profile, "AUCLST") > 0
    for code in ("LAMZ", "LAMZHL", "AUCIFO", "AUCIFP", "AUCPEO", "CLFO", "VZFP"):
        assert value(profile, code) is None


def test_non_decreasing_tail_is_not_estimated():
    concentrations = {1: 1.0, 2: 3.0, 4: 2.0, 8: 2.0, 12: 2.1, 24: 2.2}
    observations = [
        {"subject": 1, "time": t, "concentration": c} for t, c in concentrations.items()
    ]
    profile = run(observations)
    assert profile["lambda_z"]["status"] == "not_estimated"
    assert profile["lambda_z"]["reason"] == "no candidate terminal set has a negative slope"
    fixed = run(observations, lambda_z={"rule": "last_points", "points": 3})
    assert fixed["lambda_z"]["reason"] == "the selected terminal points do not decline"


def test_last_points_rule_uses_exactly_the_requested_points():
    profile = run(
        rows(oral, [0.5, 1, 2, 3, 6, 12, 24, 36, 48, 72]),
        lambda_z={"rule": "last_points", "points": 3},
    )
    assert profile["lambda_z"]["times"] == [36, 48, 72]
    assert value(profile, "LAMZNPT") == 3


def test_adjusted_r2_tolerance_prefers_more_points():
    times = [0.5, 1, 2, 3, 6, 8, 12, 24, 36, 48, 72]
    loose = run(rows(oral, times))
    strict = run(rows(oral, times), lambda_z={"rule": "adjusted_r2", "tolerance": 0.0})
    assert loose["lambda_z"]["times"][0] < strict["lambda_z"]["times"][0]
    assert abs(value(strict, "LAMZ") - K) < abs(value(loose, "LAMZ") - K)


def test_duplicate_times_fail():
    observations = rows(oral, [1, 2, 2, 4, 8, 12])
    with pytest.raises(qq.QSPQuestError, match="duplicate observation time 2"):
        run(observations)


def test_blq_rows_require_an_explicit_rule():
    observations = rows(oral, [1, 2, 4, 8, 12, 24]) + [
        {"subject": 1, "time": 48.0, "blq": True}
    ]
    with pytest.raises(qq.QSPQuestError, match="trailing BLQ row at time 48"):
        run(observations)
    profile = run(observations, blq=LEADING_ZERO)
    assert value(profile, "TLST") == 24
    assert profile["rows"][-1]["role"] == "trailing_blq_excluded"


def test_leading_blq_is_zero_and_embedded_blq_fails_or_is_excluded():
    observations = [
        {"subject": 1, "time": 0.0, "blq": True},
        {"subject": 1, "time": 0.5, "blq": True},
        {"subject": 1, "time": 1.0, "concentration": 2.0},
        {"subject": 1, "time": 2.0, "concentration": 1.0},
        {"subject": 1, "time": 3.0, "blq": True},
        {"subject": 1, "time": 4.0, "concentration": 0.5},
    ]
    with pytest.raises(qq.QSPQuestError, match="embedded BLQ row at time 3"):
        run(observations, blq=LEADING_ZERO)
    rule = {**LEADING_ZERO, "embedded": "exclude"}
    profile = run(observations, blq=rule)
    assert [row["role"] for row in profile["rows"]] == [
        "predose", "blq_set_to_zero", "auc", "auc", "embedded_blq_excluded", "auc",
    ]
    assert value(profile, "AUCLST") == pytest.approx(0.5 + 2 / math.log(2))
    zero = run(observations, blq={**LEADING_ZERO, "embedded": "zero"})
    # 2 -> 0 and 0 -> 0.5 are linear segments because log needs positive values.
    assert value(zero, "AUCLST") == pytest.approx(0.5 + 1 / math.log(2) + 0.5 + 0.25)
    assert value(zero, "LAMZ") is None
    with pytest.raises(qq.QSPQuestError, match="leading BLQ row"):
        run(observations, blq={**rule, "leading": "error"})


def test_quantifiable_predose_concentration_fails():
    observations = rows(oral, [1, 2, 4, 8], predose=False) + [
        {"subject": 1, "time": 0.0, "concentration": 0.1}
    ]
    with pytest.raises(qq.QSPQuestError, match="quantifiable pre-dose concentration"):
        run(observations)


def test_iv_bolus_requires_declining_initial_concentrations():
    observations = [
        {"subject": 1, "time": 1.0, "concentration": 2.0},
        {"subject": 1, "time": 2.0, "concentration": 3.0},
        {"subject": 1, "time": 4.0, "concentration": 1.0},
    ]
    with pytest.raises(qq.QSPQuestError, match="back-extrapolate C0"):
        run(observations, route="iv_bolus")


def test_grouping_by_subject_and_occasion():
    observations = rows(oral, [1, 2, 4, 8, 12, 24]) + [
        {**row, "occasion": 2} for row in rows(bolus, [1, 2, 4, 8], predose=False)
    ]
    doses = [
        {"subject": 1, "amount": DOSE, "route": "extravascular"},
        {"subject": 1, "occasion": 2, "amount": DOSE, "route": "iv_bolus"},
    ]
    result = qq.noncompartmental_analysis(observations, doses, units=UNITS)
    assert [(p["occasion"], p["route"]) for p in result["profiles"]] == [
        (1, "extravascular"),
        (2, "iv_bolus"),
    ]
    assert {row["parameter"] for row in result["table"] if row["occasion"] == 2} >= {
        "CLO",
        "VZO",
    }
    with pytest.raises(qq.QSPQuestError, match="has no dose"):
        qq.noncompartmental_analysis(observations, doses[:1], units=UNITS)
    with pytest.raises(qq.QSPQuestError, match="more than one dose"):
        qq.noncompartmental_analysis(observations, doses + doses[:1], units=UNITS)
    with pytest.raises(qq.QSPQuestError, match="route must be one of"):
        qq.noncompartmental_analysis(
            observations[:7], [{**doses[0], "route": "iv_infusion"}], units=UNITS
        )


def test_blq_rows_must_not_carry_a_concentration():
    with pytest.raises(qq.QSPQuestError, match="must not carry a concentration"):
        run([{"subject": 1, "time": 1.0, "blq": True, "concentration": 0.05}])


def test_data_frame_blq_rows_hold_missing_concentrations():
    pd = pytest.importorskip("pandas")
    times = [0.5, 1, 2, 4, 8, 24, 36, 48, 72]
    frame = pd.DataFrame(
        rows(oral, times) + [{"subject": 1, "time": 96.0, "blq": True}]
    )
    # Missing cells: blq is NaN on quantified rows, concentration NaN on BLQ rows.
    assert frame["concentration"].isna().sum() == 2
    assert frame["blq"].isna().sum() == len(times)
    doses = pd.DataFrame([{"subject": 1, "amount": 300, "route": "extravascular"}])
    from_frame = qq.noncompartmental_analysis(
        frame, doses, units=UNITS, blq=LEADING_ZERO
    )["profiles"][0]
    from_rows = run(
        rows(oral, times) + [{"subject": 1, "time": 96.0, "blq": True}],
        blq=LEADING_ZERO,
    )
    assert from_frame["parameters"] == from_rows["parameters"]
    assert from_frame["rows"][-1] == {
        "time": 96.0, "concentration": None, "blq": True, "role": "trailing_blq_excluded"
    }
    with pytest.raises(qq.QSPQuestError, match="must be a finite number"):
        qq.noncompartmental_analysis(
            pd.DataFrame([{"subject": 1, "time": 1.0, "concentration": float("nan")}]),
            doses,
            units=UNITS,
        )


def test_numpy_scalars_are_accepted():
    np = pytest.importorskip("numpy")
    times = [0.5, 1, 2, 4, 8, 24, 36, 48, 72]
    observations = [{"subject": np.int64(1), "time": np.float64(0.0), "blq": np.bool_(True)}]
    observations += [
        {"subject": np.int64(1), "occasion": np.int32(1), "time": np.int64(t) if t == int(t)
         else np.float64(t), "concentration": np.float32(oral(t))}
        for t in times
    ]
    profile = qq.noncompartmental_analysis(
        observations,
        [{"subject": np.int64(1), "amount": np.int64(300), "route": "extravascular"}],
        units=UNITS,
        lambda_z={"rule": "last_points", "points": np.int64(4)},
    )["profiles"][0]
    assert type(profile["subject"]) is int and profile["occasion"] == 1
    assert value(profile, "LAMZNPT") == 4
    assert value(profile, "LAMZ") == pytest.approx(K, rel=1e-5)
    with pytest.raises(qq.QSPQuestError, match="must be a finite number"):
        run([{"subject": 1, "time": np.bool_(True), "concentration": 1.0}])


def test_clearance_and_volume_are_converted_to_litres():
    times = [0.5, 1, 2, 4, 8, 12, 24, 36, 48]
    # Concentrations in ng/mL (= ug/L) with the dose in mg.
    observations = [
        {"subject": 1, "time": t, "concentration": 1000 * bolus(t)} for t in times
    ]
    result = qq.noncompartmental_analysis(
        observations,
        [{"subject": 1, "amount": DOSE, "route": "iv_bolus"}],
        units={"time": "h", "concentration": "ng/mL", "amount": "mg"},
    )
    profile = result["profiles"][0]
    assert result["units"]["derived"] == "converted"
    assert profile["parameters"]["AUCIFO"]["unit"] == "ng/mL.h"
    assert value(profile, "AUCIFO") == pytest.approx(1000 * DOSE / CL, rel=1e-12)
    assert profile["parameters"]["CLO"]["unit"] == "L/h"
    assert value(profile, "CLO") == pytest.approx(CL, rel=1e-12)
    assert profile["parameters"]["VZO"]["unit"] == "L"
    assert value(profile, "VZO") == pytest.approx(V, rel=1e-12)


def test_incompatible_units_are_reported_literally():
    times = [0.5, 1, 2, 4, 8, 12, 24, 36, 48]
    observations = [{"subject": 1, "time": t, "concentration": bolus(t)} for t in times]
    # A mass dose with a molar concentration needs a molar mass: no conversion.
    result = qq.noncompartmental_analysis(
        observations,
        [{"subject": 1, "amount": DOSE, "route": "iv_bolus"}],
        units={"time": "h", "concentration": "umol/L", "amount": "mg"},
    )
    profile = result["profiles"][0]
    assert result["units"]["derived"] == "literal"
    assert profile["parameters"]["CLO"]["unit"] == "mg/(umol/L.h)"
    assert value(profile, "CLO") == pytest.approx(CL, rel=1e-12)
    assert profile["parameters"]["VZO"]["unit"] == "mg/(umol/L)"
