//! Specified random streams: SplitMix64, open-interval uniforms, Box–Muller
//! normals through `libm`, unbiased bounded integers and hashed substreams.
//!
//! The stream is part of the analysis result contract. Any change to it needs
//! a new [`RANDOM_STREAM`] label.
use serde_json::json;

pub const RANDOM_STREAM: &str = "splitmix64_box_muller_substream/v1";

#[derive(Clone, Debug)]
pub struct Stream(u64);

impl Stream {
    pub fn new(state: u64) -> Self {
        Self(state)
    }
    pub fn state(&self) -> u64 {
        self.0
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    /// Uniform in the open interval (0, 1) from the upper 53 bits.
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / ((1_u64 << 53) as f64))
    }
    /// Standard normal by the cosine branch of Box–Muller.
    pub fn normal(&mut self) -> f64 {
        libm::sqrt(-2.0 * libm::log(self.uniform()))
            * libm::cos(2.0 * std::f64::consts::PI * self.uniform())
    }
    pub fn normals(&mut self, count: usize) -> Vec<f64> {
        (0..count).map(|_| self.normal()).collect()
    }
    /// Uniform integer in `[0, n)` by rejection, without modulo bias.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        let threshold = n.wrapping_neg() % n;
        loop {
            let x = self.next_u64();
            if x >= threshold {
                return x % n;
            }
        }
    }
}

/// Seed and label of one independent work unit.
pub struct Substream {
    pub stream: Stream,
    pub id: String,
}

/// Derive the substream of `unit` from the request seed. The seed is the first
/// 64 bits of SHA-256 over the canonical JSON of the derivation record.
pub fn substream(seed: u64, analysis: &str, unit: u64) -> Substream {
    let record = json!({
        "analysis": analysis,
        "seed": seed.to_string(),
        "unit": unit.to_string(),
    });
    let id = qspquest_core::identity::canonical_hash(&record)
        .expect("derivation record is canonical JSON");
    let state = u64::from_str_radix(&id["sha256:".len().."sha256:".len() + 16], 16)
        .expect("hexadecimal digest");
    Substream {
        stream: Stream::new(state),
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_reference_values() {
        // SplitMix64 from state zero.
        let mut s = Stream::new(0);
        assert_eq!(s.next_u64(), 0xe220a8397b1dcdaf);
        assert_eq!(s.next_u64(), 0x6e789e6aa1b965f4);
    }

    #[test]
    fn bounded_integers_stay_in_range_and_cover_values() {
        let mut s = Stream::new(7);
        let mut seen = [false; 5];
        for _ in 0..200 {
            let v = s.below(5) as usize;
            seen[v] = true;
        }
        assert!(seen.iter().all(|&x| x));
    }

    #[test]
    fn substreams_are_distinct_and_reproducible() {
        let a = substream(1009, "vpc", 0);
        let b = substream(1009, "vpc", 1);
        let c = substream(1009, "vpc", 0);
        assert_ne!(a.stream.state(), b.stream.state());
        assert_eq!(a.stream.state(), c.stream.state());
        assert_eq!(a.id, c.id);
        assert!(a.id.starts_with("sha256:") && a.id.len() == 71);
    }

    #[test]
    fn normal_moments_are_plausible() {
        let mut s = Stream::new(42);
        let x = s.normals(20000);
        let mean = x.iter().sum::<f64>() / x.len() as f64;
        let var = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (x.len() - 1) as f64;
        assert!(mean.abs() < 0.03 && (var - 1.0).abs() < 0.05);
    }
}
