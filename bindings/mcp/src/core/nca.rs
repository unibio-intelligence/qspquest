//! Single-dose noncompartmental analysis of one concentration-time profile.
//!
//! A port of `_profile` in `qspquest.nca` (Python): the same CDISC parameter codes and
//! names, AUC methods (linear, linear-up/log-down), below-quantification (BLQ) rules,
//! terminal-phase rule (the largest adjusted R-squared within a tolerance, then the most
//! points; at least three points after Tmax for extravascular doses) and unit strings.
//! Every eligible terminal point is considered; nothing is thinned. Times are relative to
//! the dose.
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    IvBolus,
    Extravascular,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Linear,
    LinearUpLogDown,
}

/// What to do with a BLQ row, by its position relative to the first and last quantifiable
/// concentrations, as `blq={"leading", "embedded", "trailing"}` in `qspquest.nca`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlqAction {
    Zero,
    Error,
    Exclude,
}

#[derive(Clone, Copy, Debug)]
pub struct BlqRule {
    pub leading: BlqAction,
    pub embedded: BlqAction,
    pub trailing: BlqAction,
}

impl BlqRule {
    /// Parse {"leading": zero|error, "embedded": error|exclude|zero, "trailing": exclude|error}.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .filter(|o| {
                o.len() == 3
                    && ["leading", "embedded", "trailing"]
                        .iter()
                        .all(|k| o.contains_key(*k))
            })
            .ok_or("blq_rule must be an object with keys leading, embedded and trailing")?;
        let pick = |key: &str, allowed: &[&str]| -> Result<BlqAction, String> {
            let text = object[key].as_str().unwrap_or("");
            if !allowed.contains(&text) {
                return Err(format!(
                    "blq_rule.{key} must be one of: {}",
                    allowed.join(", ")
                ));
            }
            Ok(match text {
                "zero" => BlqAction::Zero,
                "error" => BlqAction::Error,
                _ => BlqAction::Exclude,
            })
        };
        Ok(Self {
            leading: pick("leading", &["zero", "error"])?,
            embedded: pick("embedded", &["error", "exclude", "zero"])?,
            trailing: pick("trailing", &["exclude", "error"])?,
        })
    }
    fn json(&self) -> Value {
        let name = |a: BlqAction| match a {
            BlqAction::Zero => "zero",
            BlqAction::Error => "error",
            BlqAction::Exclude => "exclude",
        };
        json!({"leading":name(self.leading),"embedded":name(self.embedded),"trailing":name(self.trailing)})
    }
}

pub struct Units<'a> {
    pub time: &'a str,
    pub concentration: &'a str,
    pub amount: Option<&'a str>,
}

pub struct Settings<'a> {
    pub route: Route,
    pub method: Method,
    /// Administered amount in `units.amount`; clearance and volume need it.
    pub dose: Option<f64>,
    pub units: Units<'a>,
    pub blq: Option<BlqRule>,
    pub min_points: usize,
    pub tolerance: f64,
    /// Return the per-row roles (as `rows` in Python); off for dense simulated grids.
    pub rows: bool,
}

/// CDISC SDTM PK parameter short names and test names, as in `qspquest.nca.PARAMETERS`.
const NAMES: [(&str, &str); 26] = [
    ("C0", "Initial Conc"),
    ("CMAX", "Max Conc"),
    ("TMAX", "Time of CMAX Observation"),
    ("CLST", "Last Nonzero Conc"),
    ("TLST", "Time of Last Nonzero Conc"),
    ("AUCLST", "AUC to Last Nonzero Conc"),
    ("LAMZ", "Lambda z"),
    ("LAMZHL", "Half-Life Lambda z"),
    ("LAMZNPT", "Number of Points for Lambda z"),
    ("LAMZLL", "Lambda z Lower Limit"),
    ("LAMZUL", "Lambda z Upper Limit"),
    ("LAMZSPN", "Lambda z Span"),
    ("R2", "R Squared"),
    ("R2ADJ", "R Squared Adjusted"),
    ("AUCIFO", "AUC Infinity Obs"),
    ("AUCIFP", "AUC Infinity Pred"),
    ("AUCPEO", "AUC %Extrapolation Obs"),
    ("AUCPEP", "AUC %Extrapolation Pred"),
    ("CLO", "Total CL Obs"),
    ("CLP", "Total CL Pred"),
    ("VZO", "Vz Obs"),
    ("VZP", "Vz Pred"),
    ("CLFO", "Total CL Obs by F"),
    ("CLFP", "Total CL Pred by F"),
    ("VZFO", "Vz Obs by F"),
    ("VZFP", "Vz Pred by F"),
];

struct Derived {
    clearance: String,
    volume: String,
    kind: &'static str,
    factors: (f64, f64),
}

/// Clearance and volume units as `_units` in qspquest.nca: L/time and L with conversion
/// factors when amount/(concentration.time) and amount/concentration convert to them;
/// otherwise the literal units (amount/volume concentrations give volume/time and volume).
fn derived_units(amount: &str, conc: &str, time: &str) -> Derived {
    use super::usability::{convert, normalize_unit};
    let auc = format!("{conc}.{time}");
    let (clearance, volume) = (format!("{amount}/({auc})"), format!("{amount}/({conc})"));
    let factor = |from: &str, to: &str| {
        convert(1.0, &normalize_unit(from), &normalize_unit(to), "NCA unit").ok()
    };
    if let (Some(c), Some(v)) = (
        factor(&clearance, &format!("L/{time}")),
        factor(&volume, "L"),
    ) {
        return Derived {
            clearance: format!("L/{time}"),
            volume: "L".into(),
            kind: "converted",
            factors: (c, v),
        };
    }
    match conc
        .strip_prefix(&format!("{amount}/"))
        .filter(|per| !per.contains('/'))
    {
        Some(per) => Derived {
            clearance: format!("{per}/{time}"),
            volume: per.into(),
            kind: "literal",
            factors: (1.0, 1.0),
        },
        None => Derived {
            clearance,
            volume,
            kind: "literal",
            factors: (1.0, 1.0),
        },
    }
}

struct Fit {
    lambda_z: f64,
    intercept: f64,
    r2: f64,
    adjusted_r2: f64,
    points: usize,
}

fn segment(t1: f64, c1: f64, t2: f64, c2: f64, method: Method) -> f64 {
    let dt = t2 - t1;
    if method == Method::LinearUpLogDown && c1 > c2 && c2 > 0.0 {
        (c1 - c2) / (c1 / c2).ln() * dt
    } else {
        (c1 + c2) / 2.0 * dt
    }
}

fn terminal(eligible: &[(f64, f64)], min_points: usize, tolerance: f64) -> Result<Fit, String> {
    let min_points = min_points.max(3);
    if eligible.len() < min_points {
        return Err(format!("fewer than {min_points} eligible terminal points"));
    }
    // Each candidate is a suffix of the same series. Update centered sums
    // as one point joins the suffix, keeping the original longest-within-tolerance rule.
    let origin = eligible.last().map_or(0.0, |row| row.0);
    let (mut mean_x, mut mean_y) = (0.0, 0.0);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    let mut fits = Vec::with_capacity(eligible.len() - min_points + 1);
    for (index, &(time, concentration)) in eligible.iter().rev().enumerate() {
        let points = index + 1;
        let n = points as f64;
        let x = time - origin;
        let y = concentration.ln();
        let dx = x - mean_x;
        let dy = y - mean_y;
        mean_x += dx / n;
        mean_y += dy / n;
        sxx += dx * (x - mean_x);
        sxy += dx * (y - mean_y);
        syy += dy * (y - mean_y);
        if points < min_points || sxx <= 0.0 || syy <= 0.0 {
            continue;
        }
        let slope = sxy / sxx;
        let intercept = mean_y - slope * (mean_x + origin);
        if !slope.is_finite() || slope >= 0.0 || !intercept.is_finite() {
            continue;
        }
        let r2 = (sxy / sxx.sqrt() / syy.sqrt()).powi(2).clamp(0.0, 1.0);
        fits.push(Fit {
            lambda_z: -slope,
            intercept,
            r2,
            adjusted_r2: 1.0 - (1.0 - r2) * (n - 1.0) / (n - 2.0),
            points,
        });
    }
    if fits.is_empty() {
        return Err("no candidate terminal set has a negative slope".into());
    }
    let best = fits
        .iter()
        .map(|f| f.adjusted_r2)
        .fold(f64::NEG_INFINITY, f64::max);
    fits.into_iter()
        .filter(|f| f.adjusted_r2 >= best - tolerance)
        .max_by_key(|f| f.points)
        .ok_or_else(|| "no terminal set selected".into())
}

/// Analyse one profile. `blq` flags rows below the limit of quantification; their
/// concentrations are ignored.
pub fn analyze(
    times: &[f64],
    concentrations: &[f64],
    blq: Option<&[bool]>,
    s: &Settings,
) -> Result<Value, String> {
    if times.len() != concentrations.len() || times.len() < 2 {
        return Err("times and concentrations need the same length, at least 2".into());
    }
    let flags: Vec<bool> = match blq {
        Some(f) if f.len() != times.len() => return Err("blq needs one flag per time".into()),
        Some(f) => f.to_vec(),
        None => vec![false; times.len()],
    };
    if times.windows(2).any(|w| w[1] <= w[0]) {
        return Err("times must increase strictly (one value per time)".into());
    }
    if times.iter().any(|v| !v.is_finite())
        || concentrations
            .iter()
            .zip(&flags)
            .any(|(c, b)| !b && (!c.is_finite() || *c < 0.0))
    {
        return Err(
            "times and quantifiable concentrations must be finite, and concentrations nonnegative"
                .into(),
        );
    }
    let mut roles: Vec<(f64, Value, bool, &str)> = Vec::new();
    for ((t, c), b) in times.iter().zip(concentrations).zip(&flags) {
        if *t <= 0.0 {
            if !b && *c != 0.0 {
                return Err(format!("a quantifiable pre-dose concentration {c} at time {t}; single-dose NCA requires pre-dose rows that are BLQ or zero"));
            }
            roles.push((*t, if *b { Value::Null } else { json!(c) }, *b, "predose"));
        }
    }
    let post: Vec<(f64, f64, bool)> = times
        .iter()
        .zip(concentrations)
        .zip(&flags)
        .filter(|((t, _), _)| **t > 0.0)
        .map(|((t, c), b)| (*t, *c, *b))
        .collect();
    let positive: Vec<usize> = (0..post.len())
        .filter(|i| !post[*i].2 && post[*i].1 > 0.0)
        .collect();
    let (Some(&first), Some(&last)) = (positive.first(), positive.last()) else {
        return Err("no positive post-dose concentration".into());
    };
    let mut used: Vec<(f64, f64)> = Vec::new();
    for (i, &(t, c, b)) in post.iter().enumerate() {
        if !b {
            roles.push((
                t,
                json!(c),
                false,
                if i > last {
                    "excluded_after_tlast"
                } else {
                    "auc"
                },
            ));
            if i <= last {
                used.push((t, c));
            }
            continue;
        }
        let position = if i < first {
            "leading"
        } else if i > last {
            "trailing"
        } else {
            "embedded"
        };
        let Some(rule) = s.blq else {
            return Err(format!(
                "a {position} BLQ row at time {t}; give blq_rule {{leading, embedded, trailing}}"
            ));
        };
        let action = match position {
            "leading" => rule.leading,
            "trailing" => rule.trailing,
            _ => rule.embedded,
        };
        match action {
            BlqAction::Error => return Err(format!("a {position} BLQ row at time {t}")),
            BlqAction::Zero => {
                if s.route == Route::IvBolus && position == "leading" {
                    return Err("a BLQ row before the first IV bolus concentration".into());
                }
                used.push((t, 0.0));
                roles.push((t, Value::Null, true, "blq_set_to_zero"));
            }
            BlqAction::Exclude => roles.push((
                t,
                Value::Null,
                true,
                match position {
                    "leading" => "leading_blq_excluded",
                    "trailing" => "trailing_blq_excluded",
                    _ => "embedded_blq_excluded",
                },
            )),
        }
    }
    let (c0, source) = match s.route {
        Route::IvBolus => {
            let declining = used.len() > 1 && used[0].1 > used[1].1 && used[1].1 > 0.0;
            if !declining {
                return Err("an IV bolus needs two declining positive concentrations to back-extrapolate C0".into());
            }
            let ((t1, c1), (t2, c2)) = (used[0], used[1]);
            (
                c1 * (c1 / c2).powf(t1 / (t2 - t1)),
                "log_linear_back_extrapolation",
            )
        }
        Route::Extravascular => (
            0.0,
            if times.contains(&0.0) {
                "predose_sample"
            } else {
                "imputed_zero"
            },
        ),
    };
    let mut curve = vec![(0.0, c0)];
    curve.extend_from_slice(&used);
    let peak = used
        .iter()
        .enumerate()
        .fold(0, |best, (i, p)| if p.1 > used[best].1 { i } else { best });
    let (tmax, cmax) = used[peak];
    let (tlast, clast) = *used.last().unwrap();
    let auclast: f64 = curve
        .windows(2)
        .map(|w| segment(w[0].0, w[0].1, w[1].0, w[1].1, s.method))
        .sum();
    let eligible: Vec<(f64, f64)> = used
        .iter()
        .copied()
        .filter(|(t, c)| *c > 0.0 && (s.route == Route::IvBolus || *t > tmax))
        .collect();
    let fit = terminal(&eligible, s.min_points, s.tolerance);

    // Units, as `_units` in qspquest.nca: AUC in the UCUM product unit; clearance and volume
    // converted to L/time and L when the units are compatible, otherwise literal.
    let (time, conc) = (s.units.time, s.units.concentration);
    let auc_unit = format!("{conc}.{time}");
    let rate = format!("1/{time}");
    let derived = s
        .units
        .amount
        .map(|amount| derived_units(amount, conc, time));
    let clearance_unit = derived.as_ref().map(|d| d.clearance.clone());
    let volume_unit = derived.as_ref().map(|d| d.volume.clone());
    let (to_clearance, to_volume) = derived.as_ref().map_or((1.0, 1.0), |d| d.factors);

    let mut values: Vec<(&str, Value)> = Vec::new();
    if s.route == Route::IvBolus {
        values.push(("C0", json!(c0)));
    }
    values.extend([
        ("CMAX", json!(cmax)),
        ("TMAX", json!(tmax)),
        ("CLST", json!(clast)),
        ("TLST", json!(tlast)),
        ("AUCLST", json!(auclast)),
    ]);
    let estimate = fit.as_ref().ok().map(|f| {
        let half_life = std::f64::consts::LN_2 / f.lambda_z;
        let predicted = (f.intercept - f.lambda_z * tlast).exp();
        let (aucifo, aucifp) = (
            auclast + clast / f.lambda_z,
            auclast + predicted / f.lambda_z,
        );
        (f, half_life, predicted, aucifo, aucifp)
    });
    let lambda_codes = [
        "LAMZ", "LAMZHL", "LAMZNPT", "LAMZLL", "LAMZUL", "LAMZSPN", "R2", "R2ADJ", "AUCIFO",
        "AUCIFP", "AUCPEO", "AUCPEP",
    ];
    match &estimate {
        Some((f, half_life, predicted, aucifo, aucifp)) => {
            let ll = eligible[eligible.len() - f.points].0;
            let ul = eligible[eligible.len() - 1].0;
            values.extend([
                ("LAMZ", json!(f.lambda_z)),
                ("LAMZHL", json!(half_life)),
                ("LAMZNPT", json!(f.points)),
                ("LAMZLL", json!(ll)),
                ("LAMZUL", json!(ul)),
                ("LAMZSPN", json!((ul - ll) / half_life)),
                ("R2", json!(f.r2)),
                ("R2ADJ", json!(f.adjusted_r2)),
                ("AUCIFO", json!(aucifo)),
                ("AUCIFP", json!(aucifp)),
                ("AUCPEO", json!(100.0 * (clast / f.lambda_z) / aucifo)),
                ("AUCPEP", json!(100.0 * (predicted / f.lambda_z) / aucifp)),
            ]);
        }
        None => values.extend(lambda_codes.iter().map(|c| (*c, Value::Null))),
    }
    let suffix = if s.route == Route::IvBolus { "" } else { "F" };
    let (clo, clp, vzo, vzp) = (
        format!("CL{suffix}O"),
        format!("CL{suffix}P"),
        format!("VZ{suffix}O"),
        format!("VZ{suffix}P"),
    );
    if let Some(amount) = s.dose.filter(|_| s.units.amount.is_some()) {
        let pick = |observed: bool, volume: bool| -> Value {
            estimate
                .as_ref()
                .map_or(Value::Null, |(f, _, _, aucifo, aucifp)| {
                    let auc = if observed { *aucifo } else { *aucifp };
                    json!(if volume {
                        amount / (f.lambda_z * auc) * to_volume
                    } else {
                        amount / auc * to_clearance
                    })
                })
        };
        values.extend([
            (clo.as_str(), pick(true, false)),
            (clp.as_str(), pick(false, false)),
            (vzo.as_str(), pick(true, true)),
            (vzp.as_str(), pick(false, true)),
        ]);
    }
    let unit_of = |code: &str| -> String {
        match code {
            "C0" | "CMAX" | "CLST" => conc.to_owned(),
            "TMAX" | "TLST" | "LAMZHL" | "LAMZLL" | "LAMZUL" => time.to_owned(),
            "AUCLST" | "AUCIFO" | "AUCIFP" => auc_unit.clone(),
            "LAMZ" => rate.clone(),
            "AUCPEO" | "AUCPEP" => "%".into(),
            "LAMZNPT" | "LAMZSPN" | "R2" | "R2ADJ" => "1".into(),
            c if c.starts_with("CL") => clearance_unit.clone().unwrap_or_default(),
            _ => volume_unit.clone().unwrap_or_default(),
        }
    };
    let mut parameters = Map::new();
    for (code, value) in values {
        let name = NAMES
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, n)| *n)
            .unwrap_or("");
        parameters.insert(
            code.into(),
            json!({"value":value,"unit":unit_of(code),"name":name}),
        );
    }
    let mut lambda = json!({"status": if estimate.is_some() {"estimated"} else {"not_estimated"},
        "reason": fit.as_ref().err(), "eligible_points": eligible.len()});
    if let Some((f, _, predicted, _, _)) = &estimate {
        let selected = &eligible[eligible.len() - f.points..];
        lambda["intercept"] = json!(f.intercept);
        lambda["predicted_clast"] = json!(predicted);
        if f.points <= 100 {
            lambda["times"] = json!(selected.iter().map(|row| row.0).collect::<Vec<_>>());
        } else {
            lambda["times_note"] = json!(format!(
                "{} terminal points from LAMZLL to LAMZUL; the list is omitted",
                f.points
            ));
        }
        for row in roles.iter_mut() {
            if selected
                .binary_search_by(|point| point.0.total_cmp(&row.0))
                .is_ok()
            {
                row.3 = "auc_and_lambda_z";
            }
        }
    }
    let mut out = json!({"route": if s.route == Route::IvBolus {"iv_bolus"} else {"extravascular"},
        "auc_method": if s.method == Method::Linear {"linear"} else {"linear_up_log_down"},
        "time_zero": {"concentration": c0, "source": source},
        "parameters": parameters, "lambda_z": lambda,
        "blq": s.blq.map(|r| r.json()),
        "method": "Single-dose NCA as qspquest.noncompartmental_analysis: CDISC parameter codes; the terminal phase is the set of last points with the largest adjusted R-squared (within the tolerance, then the most points), at least three, after Tmax for extravascular doses; BLQ rows follow blq_rule by position."});
    if let (Some(amount), Some(unit)) = (s.dose, s.units.amount) {
        out["dose"] = json!({"value": amount, "unit": unit});
    }
    out["units"] = json!({"time": time, "concentration": conc, "amount": s.units.amount, "auc": auc_unit, "rate": rate,
        "clearance": clearance_unit, "volume": volume_unit, "derived": derived.as_ref().map(|d| d.kind)});
    if s.rows {
        roles.sort_by(|a, b| a.0.total_cmp(&b.0));
        out["rows"] = json!(roles
            .iter()
            .map(|(t, c, b, role)| json!({"time":t,"concentration":c,"blq":b,"role":role}))
            .collect::<Vec<_>>());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_exponential_profile_selects_all_terminal_points() {
        let eligible: Vec<_> = (1..=15_000)
            .map(|point| {
                let time = f64::from(point) / 100.0;
                (time, 4.0 * (-0.1 * time).exp())
            })
            .collect();
        let fit = terminal(&eligible, 3, 1e-4).unwrap();
        assert_eq!(fit.points, eligible.len());
        assert!((fit.lambda_z - 0.1).abs() < 1e-12);
        assert!((fit.intercept - 4.0_f64.ln()).abs() < 1e-12);
    }

    fn settings<'a>(
        route: Route,
        dose: Option<f64>,
        units: Units<'a>,
        blq: Option<BlqRule>,
    ) -> Settings<'a> {
        Settings {
            route,
            method: Method::LinearUpLogDown,
            dose,
            units,
            blq,
            min_points: 3,
            tolerance: 1e-4,
            rows: true,
        }
    }

    #[test]
    fn matches_the_python_reference_for_an_oral_profile() {
        // Reference values from qspquest.noncompartmental_analysis on the same rows
        // (extravascular, 300 mg, linear-up/log-down, adjusted_r2 rule).
        let t = [0.0, 0.5, 1.0, 2.0, 4.0, 6.0, 8.0, 12.0, 24.0];
        let c = [0.0, 2.10, 3.30, 4.20, 3.70, 2.90, 2.20, 1.25, 0.21];
        let units = Units {
            time: "h",
            concentration: "mg/L",
            amount: Some("mg"),
        };
        let r = analyze(
            &t,
            &c,
            None,
            &settings(Route::Extravascular, Some(300.0), units, None),
        )
        .unwrap();
        let v = |k: &str| r["parameters"][k]["value"].as_f64().unwrap();
        for (code, expected) in [
            ("CMAX", 4.2),
            ("TMAX", 2.0),
            ("CLST", 0.21),
            ("TLST", 24.0),
            ("AUCLST", 38.86806510522968),
            ("LAMZ", 0.14629589124508097),
            ("LAMZHL", 4.737981187720141),
            ("LAMZNPT", 4.0),
            ("LAMZLL", 6.0),
            ("LAMZUL", 24.0),
            ("LAMZSPN", 3.7990864224307694),
            ("R2", 0.9998260829140935),
            ("R2ADJ", 0.9997391243711402),
            ("AUCIFO", 40.30351211753298),
            ("AUCIFP", 40.31382820888262),
            ("AUCPEO", 3.561592865944918),
            ("AUCPEP", 3.586270934533526),
            ("CLFO", 7.443520036793342),
            ("CLFP", 7.441615280136035),
            ("VZFO", 50.8798980849274),
            ("VZFP", 50.86687819324693),
        ] {
            assert!(
                (v(code) - expected).abs() <= 1e-12 * expected.abs().max(1.0),
                "{code}: {} vs {expected}",
                v(code)
            );
        }
        // Unit strings and names as qspquest.nca writes them.
        assert_eq!(r["parameters"]["AUCIFO"]["unit"], "mg/L.h");
        assert_eq!(r["units"]["derived"], "converted");
        assert_eq!(r["parameters"]["LAMZ"]["unit"], "1/h");
        assert_eq!(r["parameters"]["CLFO"]["unit"], "L/h");
        assert_eq!(r["parameters"]["VZFO"]["unit"], "L");
        assert_eq!(r["parameters"]["AUCIFO"]["name"], "AUC Infinity Obs");
        assert_eq!(r["time_zero"]["source"], "predose_sample");
        let ng = Units {
            time: "h",
            concentration: "ng/mL",
            amount: Some("mg"),
        };
        let r = analyze(
            &t,
            &c,
            None,
            &settings(Route::Extravascular, Some(300.0), ng, None),
        )
        .unwrap();
        // mg with ng/mL: converted to L/h and L (1 mg/(ng/mL) = 1000 L).
        assert_eq!(r["parameters"]["CLFO"]["unit"], "L/h");
        assert!(
            (r["parameters"]["CLFO"]["value"].as_f64().unwrap() - 7443.520036793342).abs() < 1e-8
        );
        assert_eq!(r["parameters"]["VZFO"]["unit"], "L");
    }

    #[test]
    fn blq_rows_follow_the_rule_by_position() {
        // Python: leading BLQ set to zero, embedded excluded, trailing excluded.
        let t = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 12.0, 24.0, 36.0];
        let c = [0.0, 2.0, 3.3, 4.2, 0.0, 2.2, 1.25, 0.21, 0.0];
        let flags = [true, false, false, false, true, false, false, false, true];
        let units = || Units {
            time: "h",
            concentration: "mg/L",
            amount: Some("mg"),
        };
        assert!(analyze(
            &t,
            &c,
            Some(&flags),
            &settings(Route::Extravascular, Some(300.0), units(), None)
        )
        .unwrap_err()
        .contains("leading BLQ row at time 0.25"));
        let rule = BlqRule {
            leading: BlqAction::Zero,
            embedded: BlqAction::Exclude,
            trailing: BlqAction::Exclude,
        };
        let r = analyze(
            &t,
            &c,
            Some(&flags),
            &settings(Route::Extravascular, Some(300.0), units(), Some(rule)),
        )
        .unwrap();
        let roles: Vec<&str> = r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles[0], "blq_set_to_zero");
        assert_eq!(roles[4], "embedded_blq_excluded");
        assert_eq!(roles[8], "trailing_blq_excluded");
        assert_eq!(r["parameters"]["TLST"]["value"], 24.0);
        assert_eq!(roles[5], "auc_and_lambda_z");
        assert_eq!(r["time_zero"]["source"], "imputed_zero");
        // Reference values from qspquest.noncompartmental_analysis on the same rows and blq
        // rule: ng/mL converts clearance and volume; nmol/L with a mass dose stays literal.
        for (conc, derived, refs) in [
            (
                "ng/mL",
                "converted",
                [
                    ("AUCLST", 37.60109905834539, "ng/mL.h"),
                    ("LAMZ", 0.14724142444521585, "1/h"),
                    ("LAMZNPT", 3.0, "1"),
                    ("AUCIFO", 39.02732812935071, "ng/mL.h"),
                    ("CLFO", 7686.921303085143, "L/h"),
                    ("VZFO", 52206.24109042913, "L"),
                ],
            ),
            (
                "nmol/L",
                "literal",
                [
                    ("AUCLST", 37.60109905834539, "nmol/L.h"),
                    ("LAMZ", 0.14724142444521585, "1/h"),
                    ("LAMZNPT", 3.0, "1"),
                    ("AUCIFO", 39.02732812935071, "nmol/L.h"),
                    ("CLFO", 7.6869213030851435, "mg/(nmol/L.h)"),
                    ("VZFO", 52.20624109042913, "mg/(nmol/L)"),
                ],
            ),
        ] {
            let units = Units {
                time: "h",
                concentration: conc,
                amount: Some("mg"),
            };
            let r = analyze(
                &t,
                &c,
                Some(&flags),
                &settings(Route::Extravascular, Some(300.0), units, Some(rule)),
            )
            .unwrap();
            assert_eq!(r["units"]["derived"], derived);
            for (code, expected, unit) in refs {
                let got = r["parameters"][code]["value"].as_f64().unwrap();
                assert!(
                    (got - expected).abs() <= 1e-12 * expected,
                    "{conc} {code}: {got} vs {expected}"
                );
                assert_eq!(r["parameters"][code]["unit"], unit, "{conc} {code}");
            }
        }
        let strict = BlqRule {
            leading: BlqAction::Zero,
            embedded: BlqAction::Error,
            trailing: BlqAction::Exclude,
        };
        assert!(analyze(
            &t,
            &c,
            Some(&flags),
            &settings(Route::Extravascular, Some(300.0), units(), Some(strict))
        )
        .is_err());
        assert!(BlqRule::parse(
            &json!({"leading":"exclude","embedded":"zero","trailing":"exclude"})
        )
        .is_err());
    }

    #[test]
    fn iv_bolus_back_extrapolates_c0_and_refuses_predose_concentrations() {
        let units = || Units {
            time: "h",
            concentration: "mg/L",
            amount: None,
        };
        let t = [1.0, 2.0, 4.0, 8.0];
        let c: Vec<f64> = t.iter().map(|x: &f64| 10.0 * (-0.2 * x).exp()).collect();
        let r = analyze(&t, &c, None, &settings(Route::IvBolus, None, units(), None)).unwrap();
        assert!((r["parameters"]["C0"]["value"].as_f64().unwrap() - 10.0).abs() < 1e-9);
        assert!((r["parameters"]["LAMZ"]["value"].as_f64().unwrap() - 0.2).abs() < 1e-9);
        assert!(r["parameters"].get("CLO").is_none());
        assert!(analyze(
            &[0.0, 1.0],
            &[1.0, 0.5],
            None,
            &settings(Route::Extravascular, None, units(), None)
        )
        .is_err());
    }

    #[test]
    fn dense_profiles_use_every_terminal_point() {
        // 2001 points of an exact exponential: lambda_z uses all eligible points (no thinning).
        let t: Vec<f64> = (1..=2001).map(|i| i as f64 * 0.01).collect();
        let c: Vec<f64> = t.iter().map(|x| 5.0 * (-0.3 * x).exp()).collect();
        let r = analyze(
            &t,
            &c,
            None,
            &settings(
                Route::IvBolus,
                None,
                Units {
                    time: "h",
                    concentration: "mg/L",
                    amount: None,
                },
                None,
            ),
        )
        .unwrap();
        assert_eq!(r["parameters"]["LAMZNPT"]["value"], 2001);
        assert!((r["parameters"]["LAMZ"]["value"].as_f64().unwrap() - 0.3).abs() < 1e-9);
    }
}
