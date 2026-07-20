use dtw_rs::{Distance, Midpoint, Solution, fastdtw};
use libafl::prelude::*;

use crate::coverage::*;
use crate::feedback::*;
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

pub(crate) fn coverage_distance(a: &Coverages, b: &Coverages) -> f64 {
    let cover_names = a.names();
    if cover_names.is_empty() {
        return 0.0;
    }

    cover_names
        .into_iter()
        .map(|cover_name| {
            let cover_len = a.len(&cover_name);
            if cover_len == 0 {
                return 0.0;
            }

            let a_counts = a.covered_counts(&cover_name);
            let b_counts = b.covered_counts(&cover_name);
            log_euclidean_distance(&a_counts, &b_counts) / (cover_len as f64).sqrt()
        })
        .sum::<f64>()
        / a.names().len() as f64
}

pub(crate) fn state_trackers_distance(a: &StateTrackers, b: &StateTrackers) -> f64 {
    let a_core_state = a
        .arch_int_reg_tracker
        .iter()
        .zip(a.csr_tracker.iter())
        .map(|(arch, csr)| CoreStateRef {
            arch_int_reg_state: arch,
            csr_state: csr,
        })
        .collect::<Vec<_>>();
    let b_core_state = b
        .arch_int_reg_tracker
        .iter()
        .zip(b.csr_tracker.iter())
        .map(|(arch, csr)| CoreStateRef {
            arch_int_reg_state: arch,
            csr_state: csr,
        })
        .collect::<Vec<_>>();

    fastdtw_distance(&a_core_state, &b_core_state, 1)
}

pub(crate) fn combine_raw_distance(
    cover_distance: f64,
    state_distance: f64,
    cover_weight: f64,
) -> f64 {
    cover_weight * cover_distance + (1.0 - cover_weight) * state_distance
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

        for &(idx, raw) in &indexed[i..j] {
            if raw == 0.0 {
                result[idx] = 0.0;
            } else {
                result[idx] = q;
            }
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct RankedCorpusCase {
    pub(crate) id: CorpusId,
    pub(crate) distance: f64,
    pub(crate) fitness: f64,
}

pub(crate) fn ranked_passed_cases<I, S>(
    state: &S,
    initial_id: CorpusId,
    cover_weight: f64,
) -> Result<Vec<RankedCorpusCase>, Error>
where
    S: HasCorpus<I>,
{
    let initial = state.corpus().get(initial_id)?.borrow();
    let initial_covers = &initial.metadata::<CoveragesMetadata>()?.covers;
    let initial_trackers = &initial.metadata::<StateTrackersMetadata>()?.trackers;

    let mut cases = Vec::new();
    for id in state.corpus().ids() {
        if id == initial_id {
            continue;
        }

        let testcase = state.corpus().get(id)?.borrow();
        if !testcase.metadata::<PassedMetadata>()?.is_passed {
            continue;
        }

        let covers = &testcase.metadata::<CoveragesMetadata>()?.covers;
        let trackers = &testcase.metadata::<StateTrackersMetadata>()?.trackers;
        let cover_distance = coverage_distance(initial_covers, covers);
        let state_distance = state_trackers_distance(initial_trackers, trackers);
        cases.push((id, cover_distance, state_distance));
    }
    drop(initial);

    let cover_distances = quantile_transform(
        &cases
            .iter()
            .map(|(_, cover_distance, _)| *cover_distance)
            .collect::<Vec<_>>(),
    );
    let state_distances = quantile_transform(
        &cases
            .iter()
            .map(|(_, _, state_distance)| *state_distance)
            .collect::<Vec<_>>(),
    );

    Ok(cases
        .into_iter()
        .zip(cover_distances)
        .zip(state_distances)
        .map(
            |(((id, _, _), cover_distance), state_distance)| RankedCorpusCase {
                id,
                distance: combine_raw_distance(cover_distance, state_distance, cover_weight),
                fitness: distance_similarity(combine_raw_distance(
                    cover_distance,
                    state_distance,
                    cover_weight,
                )),
            },
        )
        .collect())
}
