use dtw_rs::{Distance, Midpoint, Solution, fastdtw};
use ndarray::ArrayView1;

use crate::coverage::*;
use crate::state_tracker::*;

pub(crate) fn log_euclidean_distance<T>(a: &[T], b: &[T]) -> f64
where
    T: CoveragePoint,
{
    assert_eq!(a.len(), b.len());

    let dist_sq: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let dx = (x.as_u64() as f64 + 1.0).ln() - (y.as_u64() as f64 + 1.0).ln();
            dx * dx
        })
        .sum();

    dist_sq.sqrt()
}

pub(crate) fn euclidean_distance<T>(a: &[T], b: &[T]) -> f64
where
    T: CoveragePoint,
{
    assert_eq!(a.len(), b.len());

    let dist_sq: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let dx = x.as_u64() as f64 - y.as_u64() as f64;
            dx * dx
        })
        .sum();

    dist_sq.sqrt()
}

#[derive(Clone, Copy)]
pub(crate) struct CoreStateRef<'a> {
    pub arch_int_reg_state: &'a ArchIntRegState,
    pub csr_state: &'a CSRState,
}

impl<'a> Distance for CoreStateRef<'a> {
    type Output = f64;

    fn distance(&self, other: &Self) -> Self::Output {
        0.5 * (self.arch_int_reg_state.distance(&other.arch_int_reg_state)
            + self.csr_state.distance(&other.csr_state))
    }
}

impl<'a> Midpoint for CoreStateRef<'a> {
    fn midpoint(&self, _other: &Self) -> Self {
        self.clone()
    }
}

pub(crate) fn fastdtw_distance(a: &[CoreStateRef], b: &[CoreStateRef], radius: usize) -> f64 {
    let solution = fastdtw(a, b, radius);
    let path_len = solution.path().len().max(1) as f64;
    solution.distance() / path_len
}

pub(crate) fn quantile_transform(values: &[f64]) -> Vec<f64> {
    let len = values.len();

    if len <= 1 {
        return vec![0.0; len];
    }

    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();

    indexed.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut result = vec![0.0; len];

    let mut i = 0;

    while i < len {
        let mut j = i + 1;
        while j < len && indexed[j].1 == indexed[i].1 {
            j += 1;
        }

        let avg_rank = ((i + j - 1) as f64) / 2.0;

        let q = avg_rank / ((len - 1) as f64);

        for &(idx, _) in &indexed[i..j] {
            result[idx] = q;
        }

        i = j;
    }

    result
}

pub(crate) fn distance_similarity(distance: f64) -> f64 {
    1.0 / (1.0 + distance)
}

#[allow(dead_code)]
pub(crate) fn jaccard_similarity(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());

    let a = ArrayView1::from(a);
    let b = ArrayView1::from(b);

    let (intersection, union) =
        a.iter()
            .zip(b.iter())
            .fold((0usize, 0usize), |(i, u), (&a, &b)| {
                let a = a != 0;
                let b = b != 0;
                (i + (a & b) as usize, u + (a | b) as usize)
            });

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}
