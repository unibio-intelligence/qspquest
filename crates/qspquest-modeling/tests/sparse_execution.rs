use qspquest_modeling::{simulate, Error, Model, Protocol};
#[test]
fn jacobian_output_shape_is_checked_before_and_after_cache_population() {
    use qspquest_core::expression::Expr;
    use qspquest_modeling::runtime::{CompiledModel, ModelDefinition, StateDefinition};
    let compiled = CompiledModel::compile(&ModelDefinition {
        bindings: vec![],
        states: vec![StateDefinition {
            name: "x".into(),
            initial: Expr::number(1.0),
            rhs: Expr::symbol("x"),
            dose_scale: None,
        }],
    })
    .unwrap();
    let bound = compiled.bind(&[]).unwrap();
    for _ in 0..2 {
        for length in [0, 2] {
            let mut out = vec![0.0; length];
            assert_eq!(
                bound
                    .jac_mul(0.0, &[1.0], &[0.0], &[1.0], &mut out)
                    .unwrap_err()
                    .code,
                qspquest_modeling::ErrorCode::InvalidInput
            );
        }
        let mut out = [0.0];
        bound
            .jac_mul(0.0, &[1.0], &[0.0], &[2.0], &mut out)
            .unwrap();
        assert_eq!(out, [2.0]);
    }
}
struct Sparse {
    invalid_pattern: bool,
}
impl Model for Sparse {
    fn jacobian_sparsity(&self) -> Option<Vec<(usize, usize)>> {
        let mut entries = vec![(1, 0)];
        entries.extend((2..96).map(|i| (i, i)));
        if self.invalid_pattern {
            entries.push((96, 0));
        }
        Some(entries)
    }
    fn rhs_depends_on_time(&self) -> bool {
        false
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        let mut x = vec![1.0; 96];
        x[0] = 0.0;
        x[1] = 0.0;
        x
    }
    fn dose_scale(&self, _: usize) -> Option<f64> {
        None
    }
    fn rhs(&self, _: f64, x: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
        out[0] = 1.0;
        out[1] = x[0] * x[0];
        for i in 2..96 {
            out[i] = -((i + 1) as f64 / 100.0) * x[i];
        }
        Ok(())
    }
    fn jac_mul(
        &self,
        _: f64,
        x: &[f64],
        _: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        out[0] = 0.0;
        out[1] = 2.0 * x[0] * v[0];
        for i in 2..96 {
            out[i] = -((i + 1) as f64 / 100.0) * v[i];
        }
        Ok(())
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.0,
        end: 3.0,
        samples: (0..=12).map(|i| i as f64 / 4.0).collect(),
        events: vec![],
        rtol: 1e-11,
        atol: 1e-13,
    }
}
#[test]
fn large_sparse_solve_matches_independent_closed_form_including_initially_zero_coupling() {
    for row in simulate(
        &Sparse {
            invalid_pattern: false,
        },
        &protocol(),
    )
    .unwrap()
    {
        assert!((row.values[0] - row.time).abs() < 1e-9);
        assert!((row.values[1] - row.time.powi(3) / 3.0).abs() < 1e-8);
        for i in 2..96 {
            assert!((row.values[i] - (-((i + 1) as f64 / 100.0) * row.time).exp()).abs() < 1e-9);
        }
    }
}
#[test]
fn invalid_structural_indices_return_an_error() {
    assert!(simulate(
        &Sparse {
            invalid_pattern: true
        },
        &protocol()
    )
    .is_err());
}

#[test]
fn sparse_pivoting_and_fill_match_independent_dense_factorization() {
    use diffsol::{
        AdaptiveLU, Closure, LinearSolver, NalgebraMat, NalgebraNativeLU, NalgebraVec,
        ParameterisedOp, Vector,
    };
    fn solve<LS: LinearSolver<NalgebraMat<f64>>>(
        op: &impl diffsol::NonLinearOpJacobian<
            T = f64,
            V = NalgebraVec<f64>,
            M = NalgebraMat<f64>,
            C = diffsol::NalgebraContext,
        >,
        b: &NalgebraVec<f64>,
    ) -> NalgebraVec<f64> {
        let mut ls = LS::default();
        ls.set_problem(op);
        LinearSolver::set_linearisation(&mut ls, op, b, 0.0);
        ls.solve(b).unwrap()
    }
    let n = 79;
    let apply = |v: &[f64], y: &mut [f64]| {
        for i in 0..n {
            y[i] = 4.0 * v[i];
            if i + 1 < n {
                y[i] -= 1.1 * v[i + 1];
            }
            if i > 0 {
                y[i] += 0.7 * v[i - 1];
            }
        }
        y[0] -= 4.0 * v[0];
        y[n - 1] += 6.0 * v[0];
        y[2] += 0.31 * v[n - 2];
    };
    let closure = Closure::<NalgebraMat<f64>, _, _>::new(
        |x, _, _, y| apply(x, y),
        |_, _, _, v, y| apply(v, y),
        n,
        n,
        0,
        Default::default(),
    );
    let params = NalgebraVec::from_vec(vec![], Default::default());
    let op = ParameterisedOp {
        op: &closure,
        p: &params,
    };
    let b = NalgebraVec::from_vec(
        (0..n).map(|i| ((i + 1) as f64).sin()).collect(),
        Default::default(),
    );
    let sparse = solve::<AdaptiveLU>(&op, &b);
    let dense = solve::<NalgebraNativeLU<f64>>(&op, &b);
    // Exercise the faer sparse backend used by the catalog, with pivoting and fill.
    let mut faer_closure = diffsol::Closure::<diffsol::FaerSparseMat<f64>, _, _>::new(
        |x, _, _, y| apply(x, y),
        |_, _, _, v, y| apply(v, y),
        n,
        n,
        0,
        Default::default(),
    );
    let faer_params = diffsol::FaerVec::from_vec(vec![], Default::default());
    let faer_b = diffsol::FaerVec::from_vec(b.clone_as_vec(), Default::default());
    faer_closure.calculate_sparsity(&faer_b, 0.0, &faer_params);
    let faer_op = diffsol::ParameterisedOp {
        op: &faer_closure,
        p: &faer_params,
    };
    let mut faer_solver = diffsol::FaerSparseLU::<f64>::default();
    faer_solver.set_problem(&faer_op);
    LinearSolver::set_linearisation(&mut faer_solver, &faer_op, &faer_b, 0.0);
    fn solve_faer<LS: LinearSolver<diffsol::FaerSparseMat<f64>>>(
        ls: &LS,
        b: &diffsol::FaerVec<f64>,
    ) -> diffsol::FaerVec<f64> {
        ls.solve(b).unwrap()
    }
    let faer_solution = solve_faer(&faer_solver, &faer_b);
    let mut faer_residual = vec![0.0; n];
    apply(&faer_solution.clone_as_vec(), &mut faer_residual);
    for i in 0..n {
        assert!((faer_solution[i] - dense[i]).abs() < 1e-12);
        assert!((faer_residual[i] - b[i]).abs() < 1e-12);
    }
    let mut residual = vec![0.0; n];
    apply(sparse.batch_as_slice(0), &mut residual);
    for i in 0..n {
        assert!((sparse[i] - dense[i]).abs() < 1e-12);
        assert!((residual[i] - b[i]).abs() < 1e-12);
    }
}

// Three terms cancel at a scale where a different reduction tree loses the last term.
#[test]
fn short_history_cancellation_retains_column_order_and_range_semantics() {
    use diffsol::{DenseMatrix, FaerContext, FaerMat, FaerVec, Vector};
    let matrix = FaerMat::from_vec(
        1,
        5,
        vec![99.0, 18014398509481984.0, -18014398509481984.0, 1.0, 88.0],
        FaerContext::default(),
    );
    let mut y = FaerVec::from_vec(vec![f64::NAN], FaerContext::default());
    matrix.gemv_cols(1, 4, 2.0, &[1.0, 1.0, 1.0], 0.0, &mut y);
    assert_eq!(y.get_index(0), 2.0);
    matrix.gemv_cols(1, 4, 2.0, &[1.0, 1.0, 1.0], 3.0, &mut y);
    assert_eq!(y.get_index(0), 8.0);
    matrix.gemv_cols(2, 2, 2.0, &[], 0.5, &mut y);
    assert_eq!(y.get_index(0), 4.0);
}
