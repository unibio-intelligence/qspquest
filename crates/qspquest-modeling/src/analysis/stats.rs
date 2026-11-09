//! Normal distribution functions and sample quantiles.
const LOG_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

/// Natural log of the standard normal distribution function, accurate in
/// both tails. Below -37 the complementary error function underflows, so an
/// asymptotic expansion of the Mills ratio is used.
pub fn log_ndtr(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    if z < -37.0 {
        let t = -z;
        let s = 1.0 / (t * t);
        let series = 1.0 - s * (1.0 - s * (3.0 - s * (15.0 - s * (105.0 - s * 945.0))));
        -0.5 * t * t - t.ln() - LOG_SQRT_2PI + series.ln()
    } else if z > 0.0 {
        (-0.5 * libm::erfc(z / std::f64::consts::SQRT_2)).ln_1p()
    } else {
        (0.5 * libm::erfc(-z / std::f64::consts::SQRT_2)).ln()
    }
}

#[cfg(test)]
pub fn ndtr(z: f64) -> f64 {
    0.5 * libm::erfc(-z / std::f64::consts::SQRT_2)
}

pub fn log_density(z: f64) -> f64 {
    -0.5 * z * z - LOG_SQRT_2PI
}

/// Mills-type ratio `phi(u) / Phi(u)`, stable for very negative `u`.
pub fn inverse_mills_lower(u: f64) -> f64 {
    (log_density(u) - log_ndtr(u)).exp()
}

/// Inverse standard normal distribution function (Wichura, 1988, AS 241,
/// PPND16), relative accuracy about 1e-16.
pub fn ndtri(p: f64) -> f64 {
    if !(p > 0.0 && p < 1.0) {
        return if p == 0.0 {
            f64::NEG_INFINITY
        } else if p == 1.0 {
            f64::INFINITY
        } else {
            f64::NAN
        };
    }
    let q = p - 0.5;
    if q.abs() <= 0.425 {
        let r = 0.180625 - q * q;
        let num = (((((((2.509_080_928_730_122_7e3 * r + 3.343_057_558_358_812_8e4) * r
            + 6.726_577_092_700_870_1e4)
            * r
            + 4.592_195_393_154_987_1e4)
            * r
            + 1.373_169_376_550_946_1e4)
            * r
            + 1.971_590_950_306_551_4e3)
            * r
            + 1.331_416_678_917_843_8e2)
            * r
            + 3.387_132_872_796_366_6)
            * q;
        let den = ((((((5.226_495_278_852_854_6e3 * r + 2.872_908_573_572_194_3e4) * r
            + 3.930_789_580_009_271_1e4)
            * r
            + 2.121_379_430_158_659_6e4)
            * r
            + 5.394_196_021_424_751_1e3)
            * r
            + 6.871_870_074_920_579_1e2)
            * r
            + 4.231_333_070_160_091_1e1)
            * r
            + 1.0;
        return num / den;
    }
    let mut r = if q < 0.0 { p } else { 1.0 - p };
    r = (-r.ln()).sqrt();
    let value = if r <= 5.0 {
        r -= 1.6;
        let num = ((((((7.745_450_142_783_414e-4 * r + 2.272_384_498_926_918_5e-2) * r
            + 2.417_807_251_774_506e-1)
            * r
            + 1.270_458_252_452_368_4)
            * r
            + 3.647_848_324_763_204_6)
            * r
            + 5.769_497_221_460_691)
            * r
            + 4.630_337_846_156_545)
            * r
            + 1.423_437_110_749_683_6;
        let den = ((((((1.050_750_071_644_416_8e-9 * r + 5.475_938_084_995_345e-4) * r
            + 1.519_866_656_361_645_7e-2)
            * r
            + 1.481_039_764_274_800_8e-1)
            * r
            + 6.897_673_349_851e-1)
            * r
            + 1.676_384_830_183_803_8)
            * r
            + 2.053_191_626_637_759)
            * r
            + 1.0;
        num / den
    } else {
        r -= 5.0;
        let num = ((((((2.010_334_399_292_288_1e-7 * r + 2.711_555_568_743_487_6e-5) * r
            + 1.242_660_947_388_078_4e-3)
            * r
            + 2.653_218_952_657_612_4e-2)
            * r
            + 2.965_605_718_285_048_9e-1)
            * r
            + 1.784_826_539_917_291_3)
            * r
            + 5.463_784_911_164_114)
            * r
            + 6.657_904_643_501_103_8;
        let den = ((((((2.044_263_103_389_939_8e-15 * r + 1.421_511_758_316_445_9e-7) * r
            + 1.846_318_317_510_054_7e-5)
            * r
            + 7.868_691_311_456_132_6e-4)
            * r
            + 1.487_536_129_085_061_5e-2)
            * r
            + 1.369_298_809_227_358e-1)
            * r
            + 5.998_322_065_558_879e-1)
            * r
            + 1.0;
        num / den
    };
    if q < 0.0 {
        -value
    } else {
        value
    }
}

/// Upper quantile of a chi-squared distribution with one degree of freedom.
pub fn chi2_1_ppf(confidence: f64) -> f64 {
    ndtri(0.5 + 0.5 * confidence).powi(2)
}

/// Linear interpolation in NumPy's two-sided form, exact at both ends.
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    let d = b - a;
    if t >= 0.5 {
        b - d * (1.0 - t)
    } else {
        a + d * t
    }
}

/// Sample quantiles of type 7 (Hyndman and Fan, 1996) of finite values.
pub fn quantiles(values: &[f64], probabilities: &[f64]) -> Vec<f64> {
    // NumPy returns NaN for every quantile of a sample containing NaN.
    if values.iter().any(|v| v.is_nan()) {
        return vec![f64::NAN; probabilities.len()];
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    quantiles_sorted(&sorted, probabilities)
}

pub fn quantiles_sorted(sorted: &[f64], probabilities: &[f64]) -> Vec<f64> {
    let n = sorted.len();
    probabilities
        .iter()
        .map(|&p| {
            if n == 1 {
                return sorted[0];
            }
            let h = p * (n - 1) as f64;
            let lo = (h.floor() as usize).min(n - 1);
            let hi = (lo + 1).min(n - 1);
            lerp(sorted[lo], sorted[hi], h - lo as f64)
        })
        .collect()
}

pub fn quantile(values: &[f64], probability: f64) -> f64 {
    quantiles(values, &[probability])[0]
}

pub fn median(values: &[f64]) -> f64 {
    quantile(values, 0.5)
}

pub fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// Variance with `ddof` degrees of freedom removed.
pub fn variance(values: &[f64], ddof: usize) -> f64 {
    let m = mean(values);
    values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (values.len() - ddof) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_of_a_sample_with_nan_are_nan() {
        let q = quantiles(&[1.0, f64::NAN, 3.0], &[0.1, 0.5]);
        assert!(q.iter().all(|v| v.is_nan()));
        assert_eq!(quantiles(&[3.0, 1.0, 2.0], &[0.5]), vec![2.0]);
    }

    #[test]
    fn inverse_normal_reference_points_and_round_trip() {
        assert!((ndtri(0.975) - 1.959_963_984_540_054).abs() < 1e-14);
        assert!((ndtri(0.5)).abs() < 1e-16);
        assert!((ndtri(1e-10) + 6.361_340_902_404_056).abs() < 1e-12);
        assert!((chi2_1_ppf(0.95) - 3.841_458_820_694_124).abs() < 1e-12);
        for &p in &[
            1e-300,
            1e-20,
            1e-6,
            0.01,
            0.2,
            0.4,
            0.6,
            0.9,
            0.999,
            1.0 - 1e-12,
        ] {
            let z = ndtri(p);
            let back = if p < 0.5 { ndtr(z) } else { 1.0 - ndtr(-z) };
            assert!(((back - p) / p).abs() < 1e-12, "p={p} z={z} back={back}");
            if p < 0.5 {
                assert!(z < 0.0);
            }
        }
    }

    #[test]
    fn log_ndtr_is_continuous_across_branches() {
        for &z in &[-37.0 - 1e-9, -37.0, -10.0, -1.0, 0.0, 1e-12, 3.0, 30.0] {
            let v = log_ndtr(z);
            assert!(v.is_finite() && v <= 0.0);
        }
        let below = log_ndtr(-37.000_000_001);
        let above = log_ndtr(-36.999_999_999);
        assert!((below - above).abs() < 1e-6);
        assert!((log_ndtr(-1.0) - (-1.841_021_645_009_264_2)).abs() < 1e-14);
        assert!((log_ndtr(-510.0) + 0.5 * 510.0f64.powi(2)).abs() < 10.0);
    }

    #[test]
    fn type_seven_quantiles() {
        let x = [3.0, 1.0, 2.0, 4.0];
        assert_eq!(quantiles(&x, &[0.0, 0.5, 1.0]), vec![1.0, 2.5, 4.0]);
        assert!((quantile(&x, 0.1) - 1.3).abs() < 1e-15);
        assert_eq!(median(&[5.0]), 5.0);
    }
}
