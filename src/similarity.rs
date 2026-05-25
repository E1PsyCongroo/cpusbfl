use dtw_rs::{Solution, dtw, fastdtw};
use ndarray::ArrayView1;

use crate::coverage::*;
use crate::state_tracker::*;

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

pub(crate) fn fastdtw_distance<T>(
    a: &StateTracker<T>,
    b: &StateTracker<T>,
    radius: usize,
) -> Result<f64, String>
where
    T: State,
{
    let solution = fastdtw(a.as_slice(), b.as_slice(), radius);
    let path_len = solution.path().len().max(1) as f64;
    Ok(solution.distance() / path_len)
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
