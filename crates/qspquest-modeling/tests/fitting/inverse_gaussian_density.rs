use qspquest_core::{
    expression::{Expr, Function},
    units::Unit,
    ErrorCode,
};
use qspquest_modeling::{
    compile::{Program, Symbol, SymbolKind},
    language::expression,
};

fn expression() -> Expr {
    Expr::call(
        Function::InverseGaussianDensity,
        ["time", "mean", "shape"].map(Expr::symbol).to_vec(),
    )
}
fn symbols() -> Vec<Symbol> {
    ["time", "mean", "shape"]
        .iter()
        .map(|name| Symbol {
            name: (*name).into(),
            kind: if *name == "time" {
                SymbolKind::Time
            } else {
                SymbolKind::Parameter
            },
        })
        .collect()
}

#[test]
fn density_has_exact_zero_extension_and_guarded_parameter_domain() {
    let program = Program::compile(&expression(), &symbols()).unwrap();
    for time in [-100., 0., 1e-300] {
        assert_eq!(program.evaluate(&[time, 2., 4.]).unwrap(), 0.);
        for name in ["time", "mean", "shape"] {
            assert_eq!(
                Program::derivative(&expression(), &symbols(), name)
                    .unwrap()
                    .evaluate(&[time, 2., 4.])
                    .unwrap(),
                0.
            );
        }
    }
    for values in [[0., 0., 4.], [-1., 2., -1.], [1., -2., 4.]] {
        assert_eq!(
            program.evaluate(&values).unwrap_err().code,
            ErrorCode::Domain
        );
    }
    let malformed = Expr::call(Function::InverseGaussianDensity, vec![Expr::number(1.)]);
    assert_eq!(
        Program::compile(&malformed, &[]).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn density_integrates_to_one_and_matches_its_independent_cdf() {
    let program = Program::compile(&expression(), &symbols()).unwrap();
    let mean = 2.;
    let shape = 4.;
    let end: f64 = 20.;
    let intervals = 20000;
    let step = end / intervals as f64;
    let mut area = 0.;
    for i in 0..=intervals {
        let value = program.evaluate(&[i as f64 * step, mean, shape]).unwrap();
        area += value
            * if i == 0 || i == intervals {
                1.
            } else if i % 2 == 0 {
                2.
            } else {
                4.
            };
    }
    area *= step / 3.;
    let normal = |z: f64| 0.5 * libm::erfc(-z / std::f64::consts::SQRT_2);
    let cdf = normal(libm::sqrt(shape / end) * (end / mean - 1.))
        + libm::exp(2. * shape / mean) * normal(-libm::sqrt(shape / end) * (end / mean + 1.));
    assert!((area - cdf).abs() < 1e-10, "area={area}, cdf={cdf}");
    assert!(area > 0.99999);
    let peak = program.evaluate(&[mean, mean, shape]).unwrap();
    assert!((peak - libm::sqrt(shape / (2. * std::f64::consts::PI * mean.powi(3)))).abs() < 1e-14);
}

#[test]
fn density_parameter_and_clock_derivatives_match_finite_differences() {
    let p = Program::compile(&expression(), &symbols()).unwrap();
    for time in [0.2, 2., 8.] {
        for (axis, name) in ["time", "mean", "shape"].iter().enumerate() {
            let values = [time, 2., 4.];
            let h = 1e-5;
            let mut plus = values;
            plus[axis] += h;
            let mut minus = values;
            minus[axis] -= h;
            let numeric = (p.evaluate(&plus).unwrap() - p.evaluate(&minus).unwrap()) / (2. * h);
            let analytic = Program::derivative(&expression(), &symbols(), name)
                .unwrap()
                .evaluate(&values)
                .unwrap();
            assert!(
                (numeric - analytic).abs() < 1e-7 * (1. + numeric.abs()),
                "{name} at {time}: {numeric} vs {analytic}"
            );
        }
    }
}

#[test]
fn density_units_convert_clocks_and_reject_mismatched_dimensions() {
    let expr = expression();
    let bindings = |clock: &str, mean: &str| {
        symbols()
            .into_iter()
            .zip([clock, mean, "h"])
            .map(|(symbol, unit)| (symbol, Unit::parse(unit).unwrap()))
            .collect::<Vec<_>>()
    };
    let hours =
        Program::compile_with_units(&expr, &bindings("h", "h"), Unit::parse("1/h").unwrap())
            .unwrap();
    let minutes =
        Program::compile_with_units(&expr, &bindings("min", "h"), Unit::parse("1/min").unwrap())
            .unwrap();
    assert!(
        (hours.evaluate(&[2., 2., 4.]).unwrap() / 60. - minutes.evaluate(&[120., 2., 4.]).unwrap())
            .abs()
            < 1e-14
    );
    assert_eq!(
        Program::compile_with_units(&expr, &bindings("h", "mg"), Unit::parse("1/h").unwrap())
            .unwrap_err()
            .code,
        ErrorCode::Unit
    );
}

#[test]
fn density_language_round_trips_without_enabling_generic_dynamic_switches() {
    let parsed = expression::parse("inverse_gaussian_density(time, mean, shape)", 1, 1).unwrap();
    assert_eq!(
        expression::parse(&expression::format(&parsed), 1, 1).unwrap(),
        parsed
    );
    let branch = expression::parse("time > 0 ? time : 0", 1, 1).unwrap();
    assert_eq!(
        Program::compile(&branch, &symbols()).unwrap_err().code,
        ErrorCode::Unsupported
    );
}
