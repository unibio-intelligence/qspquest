//! Deterministic scalar sparse elimination for large, sparse systems. Small or
//! dense systems retain DiffSol's reusable nalgebra LU. No magnitude threshold
//! discards entries: only exact zeros are omitted, and fill-in is retained.
use crate::error::LaError;
use crate::{
    Context, LinearOp as LaLinearOp, LinearSolver, Matrix, NalgebraContext, NalgebraLU,
    NalgebraMat, NalgebraVec,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct AdaptiveLU {
    dense: NalgebraLU<f64>,
    matrix: Option<NalgebraMat<f64>>,
    sparse: Option<Factors>,
    failed: bool,
}
struct Factors {
    rows: Vec<BTreeMap<usize, f64>>,
    swaps: Vec<(usize, usize)>,
}
impl Factors {
    fn factor(mut rows: Vec<BTreeMap<usize, f64>>) -> Result<Self, LaError> {
        let n = rows.len();
        let mut swaps = Vec::new();
        for k in 0..n {
            let mut pivot = k;
            let mut best = 0.0;
            for (i, row) in rows.iter().enumerate().skip(k) {
                let v = row.get(&k).copied().unwrap_or(0.0).abs();
                if !v.is_finite() {
                    return Err(LaError::Other("nonfinite sparse factor".into()));
                }
                if v > best {
                    pivot = i;
                    best = v;
                }
            }
            if best == 0.0 {
                return Err(LaError::Other("singular sparse factor".into()));
            }
            if pivot != k {
                rows.swap(k, pivot);
                swaps.push((k, pivot));
            }
            let diagonal = rows[k][&k];
            let upper: Vec<_> = rows[k].range(k + 1..).map(|(&j, &v)| (j, v)).collect();
            for row in rows.iter_mut().skip(k + 1) {
                let Some(value) = row.get(&k).copied() else {
                    continue;
                };
                let factor = value / diagonal;
                if !factor.is_finite() {
                    return Err(LaError::Other("nonfinite sparse factor".into()));
                }
                row.insert(k, factor);
                for &(j, u) in &upper {
                    let value = row.get(&j).copied().unwrap_or(0.0) - factor * u;
                    if !value.is_finite() {
                        return Err(LaError::Other("nonfinite sparse fill".into()));
                    }
                    if value == 0.0 {
                        row.remove(&j);
                    } else {
                        row.insert(j, value);
                    }
                }
            }
        }
        Ok(Self { rows, swaps })
    }
    fn solve(&self, x: &mut [f64]) -> Result<(), LaError> {
        let n = self.rows.len();
        if x.len() != n {
            return Err(LaError::Other("sparse vector dimension mismatch".into()));
        }
        for &(i, j) in &self.swaps {
            x.swap(i, j);
        }
        for i in 0..n {
            let mut v = x[i];
            for (&j, &a) in self.rows[i].range(..i) {
                v -= a * x[j];
            }
            x[i] = v;
        }
        for i in (0..n).rev() {
            let mut v = x[i];
            for (&j, &a) in self.rows[i].range(i + 1..) {
                v -= a * x[j];
            }
            x[i] = v / self.rows[i][&i];
            if !x[i].is_finite() {
                return Err(LaError::Other("nonfinite sparse solution".into()));
            }
        }
        Ok(())
    }
    fn solve_transpose(&self, x: &mut [f64]) -> Result<(), LaError> {
        if x.len() != self.rows.len() {
            return Err(LaError::Other("sparse vector dimension mismatch".into()));
        }
        // U^T, then L^T, followed by the inverse row permutation.
        for (i, row) in self.rows.iter().enumerate() {
            x[i] /= row[&i];
            for (&j, &a) in row.range(i + 1..) {
                x[j] -= a * x[i];
            }
        }
        for (i, row) in self.rows.iter().enumerate().rev() {
            for (&j, &a) in row.range(..i) {
                x[j] -= a * x[i];
            }
        }
        for &(i, j) in self.swaps.iter().rev() {
            x.swap(i, j);
        }
        if x.iter().any(|v| !v.is_finite()) {
            return Err(LaError::Other("nonfinite sparse solution".into()));
        }
        Ok(())
    }
}
impl LinearSolver<NalgebraMat<f64>> for AdaptiveLU {
    fn set_sparsity<
        C: LaLinearOp<T = f64, V = NalgebraVec<f64>, M = NalgebraMat<f64>, C = NalgebraContext>,
    >(
        &mut self,
        op: &C,
    ) {
        self.dense.set_sparsity(op);
        self.matrix = if op.nrows() >= 64 && op.nrows() == op.ncols() && op.context().nbatch() == 1
        {
            Some(NalgebraMat::new_from_sparsity(
                op.nrows(),
                op.ncols(),
                op.sparsity(),
                *op.context(),
            ))
        } else {
            None
        };
        self.sparse = None;
        self.failed = false;
    }
    fn set_linearisation<
        C: LaLinearOp<T = f64, V = NalgebraVec<f64>, M = NalgebraMat<f64>, C = NalgebraContext>,
    >(
        &mut self,
        op: &C,
    ) {
        self.failed = false;
        self.sparse = None;
        if let Some(matrix) = self.matrix.as_mut() {
            op.matrix_inplace(matrix);
            let n = op.nrows();
            let mut rows = vec![BTreeMap::new(); n];
            let mut count = 0;
            for (i, row) in rows.iter_mut().enumerate() {
                for j in 0..n {
                    let v = matrix.data[(i, j)];
                    if v != 0.0 {
                        row.insert(j, v);
                        count += 1;
                    }
                }
            }
            if count <= n * n / 5 {
                match Factors::factor(rows) {
                    Ok(f) => self.sparse = Some(f),
                    Err(_) => self.failed = true,
                }
                return;
            }
        }
        self.dense.set_linearisation(op);
    }
    fn solve_in_place(&self, state: &mut NalgebraVec<f64>) -> Result<(), LaError> {
        if self.failed {
            return Err(LaError::Other("sparse factorization failed".into()));
        }
        if let Some(f) = &self.sparse {
            for batch in state.data.as_mut_slice().chunks_mut(f.rows.len()) {
                f.solve(batch)?;
            }
            Ok(())
        } else {
            self.dense.solve_in_place(state)
        }
    }
    fn solve_transpose_in_place(&self, state: &mut NalgebraVec<f64>) -> Result<(), LaError> {
        if self.failed {
            return Err(LaError::Other("sparse factorization failed".into()));
        }
        if let Some(f) = &self.sparse {
            for batch in state.data.as_mut_slice().chunks_mut(f.rows.len()) {
                f.solve_transpose(batch)?;
            }
            Ok(())
        } else {
            self.dense.solve_transpose_in_place(state)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pivot_fill_and_residual_agree_with_independent_dense_lu() {
        use crate::Vector;
        let n = 79;
        let mut a = NalgebraMat::zeros(n, n, Default::default());
        for i in 0..n {
            a.data[(i, i)] = 4.0;
            if i + 1 < n {
                a.data[(i, i + 1)] = -1.1;
                a.data[(i + 1, i)] = 0.7;
            }
        }
        a.data[(0, 0)] = 0.0;
        a.data[(n - 1, 0)] = 6.0;
        a.data[(2, n - 2)] = 0.31;
        let rows = (0..n)
            .map(|i| {
                (0..n)
                    .filter_map(|j| {
                        let v = a.data[(i, j)];
                        (v != 0.0).then_some((j, v))
                    })
                    .collect()
            })
            .collect();
        let factor = Factors::factor(rows).unwrap();
        let b: Vec<_> = (0..n).map(|i| ((i + 1) as f64).sin()).collect();
        let mut x = b.clone();
        factor.solve(&mut x).unwrap();
        let dense = a
            .data
            .clone()
            .lu()
            .solve(&NalgebraVec::from_vec(b.clone(), Default::default()).data)
            .unwrap();
        let mut xt = b.clone();
        factor.solve_transpose(&mut xt).unwrap();
        for i in 0..n {
            let residual_t = (0..n).map(|j| a.data[(j, i)] * xt[j]).sum::<f64>() - b[i];
            assert!(residual_t.abs() < 1e-12);
            assert!((x[i] - dense[i]).abs() < 1e-12);
            let residual = (0..n).map(|j| a.data[(i, j)] * x[j]).sum::<f64>() - b[i];
            assert!(residual.abs() < 1e-12);
        }
    }
    #[test]
    fn singular_and_nonfinite_fail_closed() {
        assert!(Factors::factor(vec![BTreeMap::new(); 2]).is_err());
        assert!(Factors::factor(vec![BTreeMap::from([(0, f64::NAN)])]).is_err());
    }
}
