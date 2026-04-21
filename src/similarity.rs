use ndarray::ArrayView1;
use dtw_rs::{Solution,fastdtw};

use crate::state_tracker::StateTracker;

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

pub(crate) fn euclidean_distance(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());

    let a = ArrayView1::from(a);
    let b = ArrayView1::from(b);

    let dist_sq: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum();

    dist_sq.sqrt()
}

pub(crate) fn distance_similarity(a: &[u8], b: &[u8]) -> f64 {
    1.0 / (1.0 + euclidean_distance(a, b))
}

pub(crate) fn fastdtw_similarity(
    a: &StateTracker,
    b: &StateTracker,
    radius: usize,
) -> Result<f64, String> {
    if a.state_size() != b.state_size() {
        return Err(format!(
            "state size mismatch : {} vs {}",
            a.state_size(),
            b.state_size(),
        ));
    }
    let solution = fastdtw(a.as_slice(), b.as_slice(), radius);
    let path_len = solution.path().len().max(1) as f64;
    let normalized_distance = solution.distance() / path_len;
    Ok((1.0 / (1.0 + normalized_distance)).clamp(0.0, 1.0))
}
