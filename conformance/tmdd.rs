//! Independently authored synthetic mass-action TMDD benchmark.
use qspquest_modeling::{Error, Model, Protocol};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tmdd {
    pub vc: f64,
    pub vp: f64,
    pub cl: f64,
    pub q: f64,
    pub kon: f64,
    pub koff: f64,
    pub kint: f64,
    pub kdeg: f64,
    pub r0: f64,
}
impl Tmdd {
    pub fn validate(&self) -> Result<(), Error> {
        if [self.vc, self.vp]
            .iter()
            .any(|x| !x.is_finite() || *x <= 0.0)
            || [
                self.cl, self.q, self.kon, self.koff, self.kint, self.kdeg, self.r0,
            ]
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err("invalid TMDD parameter".into());
        }
        Ok(())
    }
}
impl Model for Tmdd {
    fn validate(&self) -> Result<(), Error> {
        Tmdd::validate(self)
    }
    fn initial(&self) -> Vec<f64> {
        vec![0.0, 0.0, self.r0, 0.0, 0.0, 0.0, 0.0]
    }
    fn dose_scale(&self, state: usize) -> Option<f64> {
        match state {
            0 => Some(1.0 / self.vc),
            1 => Some(1.0 / self.vp),
            _ => None,
        }
    }
    fn rhs(&self, _time: f64, x: &[f64], rate: &[f64], y: &mut [f64]) -> Result<(), Error> {
        let b = self.kon * x[0] * x[2] - self.koff * x[3];
        let exchange = self.q * (x[0] - x[1]);
        y[0] = rate[0] - (self.cl * x[0] + exchange) / self.vc - b;
        y[1] = rate[1] + exchange / self.vp;
        y[2] = self.kdeg * (self.r0 - x[2]) - b;
        y[3] = b - self.kint * x[3];
        y[4] = self.cl * x[0] + self.vc * self.kint * x[3];
        y[5] = self.kdeg * x[2] + self.kint * x[3];
        y[6] = self.kdeg * self.r0;
        Ok(())
    }
    fn jac_mul(
        &self,
        _time: f64,
        x: &[f64],
        _rates: &[f64],
        v: &[f64],
        y: &mut [f64],
    ) -> Result<(), Error> {
        let b = self.kon * (x[2] * v[0] + x[0] * v[2]) - self.koff * v[3];
        let exchange = self.q * (v[0] - v[1]);
        y[0] = -(self.cl * v[0] + exchange) / self.vc - b;
        y[1] = exchange / self.vp;
        y[2] = -self.kdeg * v[2] - b;
        y[3] = b - self.kint * v[3];
        y[4] = self.cl * v[0] + self.vc * self.kint * v[3];
        y[5] = self.kdeg * v[2] + self.kint * v[3];
        y[6] = 0.0;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub model: Tmdd,
    pub protocol: Protocol,
}
