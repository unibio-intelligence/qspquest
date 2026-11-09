//! Small dense linear algebra for analysis problems of dimension at most 16.
//!
//! Matrices are row vectors. Arithmetic order is fixed, so native and WASM
//! builds evaluate the same operations in the same order.
pub type Matrix = Vec<Vec<f64>>;

pub fn identity(n: usize) -> Matrix {
    (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect()
}

pub fn mat_vec(a: &Matrix, x: &[f64]) -> Vec<f64> {
    a.iter()
        .map(|row| row.iter().zip(x).map(|(a, b)| a * b).sum())
        .collect()
}

pub fn transpose(a: &Matrix) -> Matrix {
    if a.is_empty() {
        return Vec::new();
    }
    (0..a[0].len())
        .map(|j| a.iter().map(|row| row[j]).collect())
        .collect()
}

pub fn mat_mul(a: &Matrix, b: &Matrix) -> Matrix {
    let cols = b.first().map_or(0, Vec::len);
    a.iter()
        .map(|row| {
            (0..cols)
                .map(|j| row.iter().zip(b).map(|(x, r)| x * r[j]).sum())
                .collect()
        })
        .collect()
}

pub fn quadratic_form(x: &[f64], a: &Matrix, y: &[f64]) -> f64 {
    x.iter()
        .zip(a)
        .map(|(xi, row)| xi * row.iter().zip(y).map(|(a, b)| a * b).sum::<f64>())
        .sum()
}

pub fn is_symmetric(a: &Matrix, n: usize, tolerance: f64) -> bool {
    a.len() == n
        && a.iter().all(|row| row.len() == n)
        && (0..n).all(|i| (0..n).all(|j| (a[i][j] - a[j][i]).abs() <= tolerance))
}

/// Lower Cholesky factor, or `None` when the matrix is not positive definite.
pub fn cholesky(a: &Matrix) -> Option<Matrix> {
    let n = a.len();
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[i][j];
            for k in 0..j {
                sum -= l[i][k] * l[j][k];
            }
            if i == j {
                if !(sum > 0.0) || !sum.is_finite() {
                    return None;
                }
                l[i][i] = sum.sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    Some(l)
}

/// Solve `L L^T x = b` for a lower Cholesky factor.
pub fn cholesky_solve(l: &Matrix, b: &[f64]) -> Vec<f64> {
    let n = l.len();
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = b[i];
        for k in 0..i {
            s -= l[i][k] * y[k];
        }
        y[i] = s / l[i][i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = y[i];
        for k in i + 1..n {
            s -= l[k][i] * x[k];
        }
        x[i] = s / l[i][i];
    }
    x
}

/// Inverse of a symmetric positive definite matrix.
pub fn inverse_spd(a: &Matrix) -> Option<Matrix> {
    let l = cholesky(a)?;
    let n = a.len();
    let mut columns = Vec::with_capacity(n);
    for j in 0..n {
        let mut e = vec![0.0; n];
        e[j] = 1.0;
        columns.push(cholesky_solve(&l, &e));
    }
    let mut inverse = transpose(&columns);
    for i in 0..n {
        for j in 0..i {
            let v = 0.5 * (inverse[i][j] + inverse[j][i]);
            inverse[i][j] = v;
            inverse[j][i] = v;
        }
    }
    Some(inverse)
}

/// Log determinant of a symmetric positive definite matrix.
pub fn log_det_spd(a: &Matrix) -> Option<f64> {
    let l = cholesky(a)?;
    Some(
        2.0 * l
            .iter()
            .enumerate()
            .map(|(i, row)| row[i].ln())
            .sum::<f64>(),
    )
}

/// Numerical rank by Householder QR with column pivoting.
pub fn rank(a: &Matrix) -> usize {
    let m = a.len();
    if m == 0 {
        return 0;
    }
    let n = a[0].len();
    let mut r = a.clone();
    let mut norms: Vec<f64> = (0..n)
        .map(|j| r.iter().map(|row| row[j] * row[j]).sum::<f64>())
        .collect();
    let mut first = None;
    let mut rank = 0;
    for k in 0..m.min(n) {
        let (pivot, _) = norms
            .iter()
            .enumerate()
            .skip(k)
            .fold(
                (k, -1.0),
                |best, (j, &v)| if v > best.1 { (j, v) } else { best },
            );
        for row in r.iter_mut() {
            row.swap(k, pivot);
        }
        norms.swap(k, pivot);
        let alpha = r[k..].iter().map(|row| row[k] * row[k]).sum::<f64>().sqrt();
        let scale = *first.get_or_insert(alpha);
        if alpha <= scale * (m.max(n) as f64) * f64::EPSILON || alpha == 0.0 {
            break;
        }
        rank += 1;
        let sign = if r[k][k] >= 0.0 { 1.0 } else { -1.0 };
        let mut v: Vec<f64> = r[k..].iter().map(|row| row[k]).collect();
        v[0] += sign * alpha;
        let vv = v.iter().map(|x| x * x).sum::<f64>();
        for j in k..n {
            let dot = v
                .iter()
                .zip(&r[k..])
                .map(|(vi, row)| vi * row[j])
                .sum::<f64>();
            let f = 2.0 * dot / vv;
            for (i, vi) in v.iter().enumerate() {
                r[k + i][j] -= f * vi;
            }
        }
        for j in k + 1..n {
            norms[j] = r[k + 1..].iter().map(|row| row[j] * row[j]).sum::<f64>();
        }
    }
    rank
}

/// Bounded least squares from normal equations `H x = g` with `H = A^T A`
/// positive definite, by a primal active-set method (the bounded-variable
/// least-squares strategy of Stark and Parker, 1995). Returns `None` when the
/// iteration limit is reached.
pub fn bounded_least_squares(
    h: &Matrix,
    g: &[f64],
    lower: &[f64],
    upper: &[f64],
    max_iterations: usize,
) -> Option<Vec<f64>> {
    let n = g.len();
    let mut x: Vec<f64> = (0..n)
        .map(|i| {
            if lower[i] == upper[i] {
                lower[i]
            } else {
                0.5 * (lower[i] + upper[i])
            }
        })
        .collect();
    let mut free: Vec<bool> = (0..n).map(|i| lower[i] < upper[i]).collect();
    let scale = h
        .iter()
        .enumerate()
        .map(|(i, row)| row[i])
        .fold(0.0, f64::max)
        .max(1.0);
    for _ in 0..max_iterations {
        let idx: Vec<usize> = (0..n).filter(|&i| free[i]).collect();
        if !idx.is_empty() {
            let sub: Matrix = idx
                .iter()
                .map(|&i| idx.iter().map(|&j| h[i][j]).collect())
                .collect();
            let rhs: Vec<f64> = idx
                .iter()
                .map(|&i| {
                    g[i] - (0..n)
                        .filter(|&j| !free[j])
                        .map(|j| h[i][j] * x[j])
                        .sum::<f64>()
                })
                .collect();
            let l = cholesky(&sub)?;
            let z = cholesky_solve(&l, &rhs);
            let feasible = idx
                .iter()
                .zip(&z)
                .all(|(&i, &zi)| zi >= lower[i] && zi <= upper[i]);
            if !feasible {
                let mut alpha = 1.0f64;
                for (&i, &zi) in idx.iter().zip(&z) {
                    if zi < lower[i] {
                        alpha = alpha.min((lower[i] - x[i]) / (zi - x[i]));
                    } else if zi > upper[i] {
                        alpha = alpha.min((upper[i] - x[i]) / (zi - x[i]));
                    }
                }
                let alpha = alpha.clamp(0.0, 1.0);
                for (&i, &zi) in idx.iter().zip(&z) {
                    x[i] += alpha * (zi - x[i]);
                    let tol = 1e-14 * (upper[i] - lower[i]).abs().max(1.0);
                    if x[i] <= lower[i] + tol {
                        x[i] = lower[i];
                        free[i] = false;
                    } else if x[i] >= upper[i] - tol {
                        x[i] = upper[i];
                        free[i] = false;
                    }
                }
                continue;
            }
            for (&i, &zi) in idx.iter().zip(&z) {
                x[i] = zi;
            }
        }
        // Negative half-gradient of the objective; positive means increasing helps.
        let w: Vec<f64> = (0..n)
            .map(|i| g[i] - h[i].iter().zip(&x).map(|(a, b)| a * b).sum::<f64>())
            .collect();
        let mut best: Option<(usize, f64)> = None;
        for i in 0..n {
            if free[i] || lower[i] == upper[i] {
                continue;
            }
            let violation = if x[i] == lower[i] { w[i] } else { -w[i] };
            if violation > 1e-12 * scale && best.is_none_or(|(_, v)| violation > v) {
                best = Some((i, violation));
            }
        }
        match best {
            None => return Some(x),
            Some((i, _)) => free[i] = true,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cholesky_inverse_and_log_determinant() {
        let a = vec![
            vec![4.0, 2.0, 0.6],
            vec![2.0, 2.0, 0.5],
            vec![0.6, 0.5, 3.0],
        ];
        let inv = inverse_spd(&a).unwrap();
        let product = mat_mul(&a, &inv);
        for i in 0..3 {
            for j in 0..3 {
                assert!((product[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-14);
            }
        }
        let det = 4.0 * (2.0 * 3.0 - 0.25) - 2.0 * (2.0 * 3.0 - 0.3) + 0.6 * (1.0 - 1.2);
        assert!((log_det_spd(&a).unwrap() - f64::ln(det)).abs() < 1e-14);
        assert!(cholesky(&vec![vec![1.0, 2.0], vec![2.0, 1.0]]).is_none());
    }

    #[test]
    fn rank_detects_dependent_columns() {
        let a = vec![
            vec![1.0, 2.0, 3.0],
            vec![1.0, 2.0, 1.0],
            vec![1.0, 2.0, 0.0],
        ];
        assert_eq!(rank(&a), 2);
        assert_eq!(rank(&identity(3)), 3);
    }

    #[test]
    fn bounded_least_squares_matches_known_solutions() {
        // min (x-3)^2 + (y+1)^2 with x in [0,2], y in [0,5] -> (2, 0).
        let h = identity(2);
        let x = bounded_least_squares(&h, &[3.0, -1.0], &[0.0, 0.0], &[2.0, 5.0], 100).unwrap();
        assert_eq!(x, vec![2.0, 0.0]);
        // Coupled problem with an interior solution.
        let h = vec![vec![2.0, 1.0], vec![1.0, 2.0]];
        let x = bounded_least_squares(&h, &[1.0, 1.0], &[-5.0, -5.0], &[5.0, 5.0], 100).unwrap();
        assert!((x[0] - 1.0 / 3.0).abs() < 1e-14 && (x[1] - 1.0 / 3.0).abs() < 1e-14);
        // Coupled problem with one active bound: min over y>=0.5.
        let x = bounded_least_squares(&h, &[1.0, 0.0], &[-5.0, 0.5], &[5.0, 5.0], 100).unwrap();
        assert!((x[1] - 0.5).abs() < 1e-15 && (x[0] - 0.25).abs() < 1e-14);
    }
}
