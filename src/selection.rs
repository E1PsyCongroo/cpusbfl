use std::{io::Write, path::Path};

use clap::ValueEnum;
use libafl::prelude::*;
use rand::seq::SliceRandom;

use crate::coverage::cover_point_name;
use crate::fuzzer::*;
use crate::reduce::*;
use crate::similarity::*;
use crate::utils::*;

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub(crate) enum Selection {
    Random,
    Sort,
    Diverse,
}

impl Default for Selection {
    fn default() -> Self {
        Self::Sort
    }
}

struct PassedCase {
    id: usize,
    input: BytesInput,
    metadata: CaseMetadata,
    fail_cover_distance: f64,
    fail_state_distance: f64,
    fail_distance: f64,
}

#[derive(Debug)]
struct PackedDistanceMatrix {
    size: usize,
    values: Vec<f64>,
}

impl PackedDistanceMatrix {
    fn new(size: usize, values: Vec<f64>) -> Result<Self, String> {
        let expected_len = size
            .checked_mul(size.saturating_sub(1))
            .and_then(|len| len.checked_div(2))
            .ok_or_else(|| format!("Distance matrix size overflow: {size}"))?;

        if values.len() != expected_len {
            return Err(format!(
                "Invalid packed distance matrix length: expected {expected_len}, got {}",
                values.len()
            ));
        }

        Ok(Self { size, values })
    }

    fn get(&self, mut i: usize, mut j: usize) -> f64 {
        assert!(i < self.size && j < self.size);
        if i == j {
            return 0.0;
        }
        if i > j {
            std::mem::swap(&mut i, &mut j);
        }

        let index = i * (2 * self.size - i - 1) / 2 + (j - i - 1);
        self.values[index]
    }
}

#[derive(Debug, Clone, Copy)]
struct SelectionDecision {
    index: usize,
    min_pass_distance: f64,
    score: f64,
}

#[derive(Debug)]
struct RwmfcGroup {
    weights: Vec<f64>,
    covered: Vec<bool>,
    total_weight: f64,
    total_fail_points: usize,
    reachable_fail_points: usize,
}

fn fail_point_difficulty_weight(pass_count: usize, pass_cover_count: usize) -> f64 {
    assert!(pass_cover_count <= pass_count);
    1.0 + ((pass_count + 1) as f64 / (pass_cover_count + 1) as f64).ln()
}

impl RwmfcGroup {
    fn new(fail_bits: &[bool], pass_cover_counts: &[usize], pass_count: usize) -> Self {
        assert_eq!(fail_bits.len(), pass_cover_counts.len());

        let mut weights = vec![0.0; fail_bits.len()];
        let mut total_weight = 0.0;
        let mut total_fail_points = 0;
        let mut reachable_fail_points = 0;

        for (index, (&fail_covered, &pass_cover_count)) in
            fail_bits.iter().zip(pass_cover_counts).enumerate()
        {
            if !fail_covered {
                continue;
            }

            total_fail_points += 1;
            if pass_cover_count == 0 {
                continue;
            }

            reachable_fail_points += 1;
            let weight = fail_point_difficulty_weight(pass_count, pass_cover_count);
            weights[index] = weight;
            total_weight += weight;
        }

        Self {
            covered: vec![false; fail_bits.len()],
            weights,
            total_weight,
            total_fail_points,
            reachable_fail_points,
        }
    }

    fn add_selected_case(&mut self, pass_bits: &[bool]) -> (f64, usize) {
        assert_eq!(self.weights.len(), pass_bits.len());

        let mut new_weight = 0.0;
        let mut new_points = 0;
        for (index, &pass_covered) in pass_bits.iter().enumerate() {
            if pass_covered && self.weights[index] > 0.0 && !self.covered[index] {
                self.covered[index] = true;
                new_weight += self.weights[index];
                new_points += 1;
            }
        }

        let new_fraction = if self.total_weight > 0.0 {
            new_weight / self.total_weight
        } else {
            0.0
        };
        (new_fraction, new_points)
    }

    fn covered_reachable_points(&self) -> usize {
        self.covered
            .iter()
            .zip(&self.weights)
            .filter(|(covered, weight)| **covered && **weight > 0.0)
            .count()
    }
}

#[derive(Debug)]
struct PassSelectionMetric {
    rank: usize,
    corpus_id: usize,
    distance: f64,
    marginal_rwmfc_percent: f64,
    cumulative_rwmfc_percent: f64,
    new_reachable_fail_points: usize,
}

#[derive(Debug)]
struct FailPointDifficulty {
    cover_name: String,
    point_index: usize,
    pass_cover_count: usize,
    candidate_pass_count: usize,
    difficulty_weight: f64,
    covered_by_selected_pass: bool,
}

impl FailPointDifficulty {
    fn reachable(&self) -> bool {
        self.pass_cover_count > 0
    }
}

#[derive(Debug)]
struct PassSelectionReport {
    cases: Vec<PassSelectionMetric>,
    fail_points: Vec<FailPointDifficulty>,
    final_rwmfc_percent: f64,
    total_fail_points: usize,
    reachable_fail_points: usize,
    selected_covered_reachable_fail_points: usize,
    reachability_percent: f64,
}

fn calculate_pass_selection_report(
    init_metadata: &CaseMetadata,
    passed_cases: &[PassedCase],
    decisions: &[SelectionDecision],
) -> PassSelectionReport {
    let mut cover_names = init_metadata.covers.names();
    cover_names.sort();

    let mut groups = Vec::with_capacity(cover_names.len());
    let mut fail_points = Vec::new();
    for cover_name in &cover_names {
        let fail_bits = init_metadata.covers.covered_bits(cover_name);
        let mut pass_cover_counts = vec![0; fail_bits.len()];
        let mut selected_pass_covered = vec![false; fail_bits.len()];

        for case in passed_cases {
            let pass_bits = case.metadata.covers.covered_bits(cover_name);
            assert_eq!(fail_bits.len(), pass_bits.len());
            for (index, (&fail_covered, pass_covered)) in
                fail_bits.iter().zip(pass_bits).enumerate()
            {
                if fail_covered && pass_covered {
                    pass_cover_counts[index] += 1;
                }
            }
        }
        for decision in decisions {
            let pass_bits = passed_cases[decision.index]
                .metadata
                .covers
                .covered_bits(cover_name);
            assert_eq!(fail_bits.len(), pass_bits.len());
            for (selected_covered, pass_covered) in selected_pass_covered.iter_mut().zip(pass_bits)
            {
                *selected_covered |= pass_covered;
            }
        }

        for (point_index, (&fail_covered, &pass_cover_count)) in
            fail_bits.iter().zip(&pass_cover_counts).enumerate()
        {
            if fail_covered {
                fail_points.push(FailPointDifficulty {
                    cover_name: cover_name.clone(),
                    point_index,
                    pass_cover_count,
                    candidate_pass_count: passed_cases.len(),
                    difficulty_weight: fail_point_difficulty_weight(
                        passed_cases.len(),
                        pass_cover_count,
                    ),
                    covered_by_selected_pass: selected_pass_covered[point_index],
                });
            }
        }

        groups.push(RwmfcGroup::new(
            &fail_bits,
            &pass_cover_counts,
            passed_cases.len(),
        ));
    }

    let scorable_group_count = groups
        .iter()
        .filter(|group| group.total_weight > 0.0)
        .count();
    let total_fail_points = groups.iter().map(|group| group.total_fail_points).sum();
    let reachable_fail_points = groups.iter().map(|group| group.reachable_fail_points).sum();
    let reachability_percent = if total_fail_points > 0 {
        100.0 * reachable_fail_points as f64 / total_fail_points as f64
    } else {
        0.0
    };

    let mut cumulative_rwmfc_percent = 0.0;
    let mut cases = Vec::with_capacity(decisions.len());
    for (rank, decision) in decisions.iter().enumerate() {
        let case = &passed_cases[decision.index];
        let mut marginal_group_fraction_sum = 0.0;
        let mut new_reachable_fail_points = 0;

        for (cover_name, group) in cover_names.iter().zip(&mut groups) {
            let pass_bits = case.metadata.covers.covered_bits(cover_name);
            let (new_fraction, new_points) = group.add_selected_case(&pass_bits);
            if group.total_weight > 0.0 {
                marginal_group_fraction_sum += new_fraction;
            }
            new_reachable_fail_points += new_points;
        }

        let marginal_rwmfc_percent = if scorable_group_count > 0 {
            100.0 * marginal_group_fraction_sum / scorable_group_count as f64
        } else {
            0.0
        };
        cumulative_rwmfc_percent += marginal_rwmfc_percent;
        cases.push(PassSelectionMetric {
            rank: rank + 1,
            corpus_id: case.id,
            distance: case.fail_distance,
            marginal_rwmfc_percent,
            cumulative_rwmfc_percent,
            new_reachable_fail_points,
        });
    }

    PassSelectionReport {
        cases,
        fail_points,
        final_rwmfc_percent: cumulative_rwmfc_percent.min(100.0),
        total_fail_points,
        reachable_fail_points,
        selected_covered_reachable_fail_points: groups
            .iter()
            .map(RwmfcGroup::covered_reachable_points)
            .sum(),
        reachability_percent,
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn write_pass_selection_metrics(
    output_dir: impl AsRef<Path>,
    report: &PassSelectionReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_dir.as_ref().join("pass_selection_metrics.csv"))?;

    writeln!(
        output,
        "rank,corpus_id,distance,marginal_rwmfc_percent,cumulative_rwmfc_percent,new_reachable_fail_points"
    )?;
    for case in &report.cases {
        writeln!(
            output,
            "{},{},{:.12},{:.6},{:.6},{}",
            case.rank,
            case.corpus_id,
            case.distance,
            case.marginal_rwmfc_percent,
            case.cumulative_rwmfc_percent,
            case.new_reachable_fail_points,
        )?;
    }

    Ok(())
}

fn write_pass_selection_summary(
    output_dir: impl AsRef<Path>,
    report: &PassSelectionReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_dir.as_ref().join("pass_selection_summary.csv"))?;

    writeln!(
        output,
        "final_rwmfc_percent,total_fail_points,reachable_fail_points,selected_covered_reachable_fail_points,reachability_percent"
    )?;
    writeln!(
        output,
        "{:.6},{},{},{},{:.6}",
        report.final_rwmfc_percent,
        report.total_fail_points,
        report.reachable_fail_points,
        report.selected_covered_reachable_fail_points,
        report.reachability_percent,
    )?;

    Ok(())
}

fn write_fail_point_difficulties(
    output_dir: impl AsRef<Path>,
    report: &PassSelectionReport,
    mut point_name: impl FnMut(&str, usize) -> String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_dir.as_ref().join("fail_point_difficulty.csv"))?;

    writeln!(
        output,
        "difficulty_rank,coverage_name,point_index,point_name,pass_cover_count,candidate_pass_count,pass_cover_rate_percent,reachable,difficulty_weight,included_in_rwmfc"
    )?;
    let mut ranked_points = report.fail_points.iter().collect::<Vec<_>>();
    ranked_points.sort_by(|a, b| {
        b.difficulty_weight
            .total_cmp(&a.difficulty_weight)
            .then_with(|| a.cover_name.cmp(&b.cover_name))
            .then_with(|| a.point_index.cmp(&b.point_index))
    });
    for (rank, point) in ranked_points.into_iter().enumerate() {
        let pass_cover_rate_percent = if point.candidate_pass_count > 0 {
            100.0 * point.pass_cover_count as f64 / point.candidate_pass_count as f64
        } else {
            0.0
        };
        writeln!(
            output,
            "{},{},{},{},{},{},{:.6},{},{:.12},{}",
            rank + 1,
            csv_field(&point.cover_name),
            point.point_index,
            csv_field(&point_name(&point.cover_name, point.point_index)),
            point.pass_cover_count,
            point.candidate_pass_count,
            pass_cover_rate_percent,
            point.reachable(),
            point.difficulty_weight,
            point.covered_by_selected_pass,
        )?;
    }

    Ok(())
}

fn is_better_diverse_candidate(
    index: usize,
    score: f64,
    diversity: f64,
    best: SelectionDecision,
    fail_distances: &[f64],
    corpus_ids: &[usize],
) -> bool {
    score
        .total_cmp(&best.score)
        .then_with(|| fail_distances[best.index].total_cmp(&fail_distances[index]))
        .then_with(|| diversity.total_cmp(&best.min_pass_distance))
        .then_with(|| corpus_ids[best.index].cmp(&corpus_ids[index]))
        .is_gt()
}

fn select_diverse_indices(
    fail_distances: &[f64],
    corpus_ids: &[usize],
    pair_distances: &PackedDistanceMatrix,
    limit: usize,
    diversity_weight: f64,
) -> Vec<SelectionDecision> {
    assert_eq!(fail_distances.len(), corpus_ids.len());
    assert_eq!(fail_distances.len(), pair_distances.size);

    let limit = limit.min(fail_distances.len());
    if limit == 0 {
        return Vec::new();
    }

    let first = (0..fail_distances.len())
        .min_by(|&a, &b| {
            fail_distances[a]
                .total_cmp(&fail_distances[b])
                .then_with(|| corpus_ids[a].cmp(&corpus_ids[b]))
        })
        .unwrap();
    let mut selected = vec![false; fail_distances.len()];
    selected[first] = true;

    let mut decisions = vec![SelectionDecision {
        index: first,
        min_pass_distance: 0.0,
        score: (1.0 - diversity_weight) * (1.0 - fail_distances[first]),
    }];
    let mut min_pass_distances = (0..fail_distances.len())
        .map(|index| pair_distances.get(index, first))
        .collect::<Vec<_>>();

    while decisions.len() < limit {
        let mut best = None;

        for index in 0..fail_distances.len() {
            if selected[index] {
                continue;
            }

            let diversity = min_pass_distances[index];
            let score = (1.0 - diversity_weight) * (1.0 - fail_distances[index])
                + diversity_weight * diversity;
            let decision = SelectionDecision {
                index,
                min_pass_distance: diversity,
                score,
            };

            if best.is_none_or(|current| {
                is_better_diverse_candidate(
                    index,
                    score,
                    diversity,
                    current,
                    fail_distances,
                    corpus_ids,
                )
            }) {
                best = Some(decision);
            }
        }

        let best = best.expect("Not enough unselected pass cases");
        selected[best.index] = true;
        decisions.push(best);

        for index in 0..fail_distances.len() {
            if !selected[index] {
                min_pass_distances[index] =
                    min_pass_distances[index].min(pair_distances.get(index, best.index));
            }
        }
    }

    decisions
}

fn select_diverse_passed_cases(
    passed_cases: &[PassedCase],
    limit: usize,
    cover_weight: f64,
    diversity_weight: f64,
    pool_factor: usize,
) -> Result<Vec<SelectionDecision>, Box<dyn std::error::Error>> {
    let mut ordered_indices = (0..passed_cases.len()).collect::<Vec<_>>();
    ordered_indices.sort_by(|&a, &b| {
        passed_cases[a]
            .fail_distance
            .total_cmp(&passed_cases[b].fail_distance)
            .then_with(|| passed_cases[a].id.cmp(&passed_cases[b].id))
    });

    let pool_size = passed_cases
        .len()
        .min(limit.saturating_mul(pool_factor.max(1)).max(limit));
    let pool_indices = &ordered_indices[..pool_size];
    let pair_count = pool_size
        .checked_mul(pool_size.saturating_sub(1))
        .and_then(|count| count.checked_div(2))
        .ok_or_else(|| format!("Pass distance matrix size overflow: {pool_size}"))?;
    let mut cover_distances = Vec::with_capacity(pair_count);
    let mut state_distances = Vec::with_capacity(pair_count);

    log::info!(
        "Diverse pass selection: total={}, pool={}, selected={}, pair_distances={}",
        passed_cases.len(),
        pool_size,
        limit,
        pair_count
    );

    for pool_i in 0..pool_size {
        for pool_j in (pool_i + 1)..pool_size {
            let case_i = &passed_cases[pool_indices[pool_i]];
            let case_j = &passed_cases[pool_indices[pool_j]];
            let cover_distance =
                coverage_distance(&case_i.metadata.covers, &case_j.metadata.covers);
            let state_distance = state_trackers_distance(
                &case_i.metadata.state_trackers,
                &case_j.metadata.state_trackers,
            );

            if !cover_distance.is_finite() || !state_distance.is_finite() {
                return Err(format!(
                    "Non-finite pass distance between corpus cases {} and {}: coverage={}, state={}",
                    case_i.id, case_j.id, cover_distance, state_distance
                )
                .into());
            }

            cover_distances.push(cover_distance);
            state_distances.push(state_distance);
        }
    }

    let cover_quantiles = quantile_transform(&cover_distances);
    let state_quantiles = quantile_transform(&state_distances);
    let pair_distances = cover_quantiles
        .into_iter()
        .zip(state_quantiles)
        .map(|(cover, state)| combine_raw_distance(cover, state, cover_weight))
        .collect::<Vec<_>>();
    let pair_distances = PackedDistanceMatrix::new(pool_size, pair_distances)?;
    let fail_distances = pool_indices
        .iter()
        .map(|&index| passed_cases[index].fail_distance)
        .collect::<Vec<_>>();
    let corpus_ids = pool_indices
        .iter()
        .map(|&index| passed_cases[index].id)
        .collect::<Vec<_>>();

    Ok(select_diverse_indices(
        &fail_distances,
        &corpus_ids,
        &pair_distances,
        limit,
        diversity_weight,
    )
    .into_iter()
    .map(|decision| SelectionDecision {
        index: pool_indices[decision.index],
        min_pass_distance: decision.min_pass_distance,
        score: decision.score,
    })
    .collect())
}

pub(crate) fn emit_top_passed_testcases(
    session: &FuzzSession,
    output: Option<impl AsRef<Path>>,

    cover_weight: f64,
    save_intermediate: bool,

    top_pass: usize,
    selection: Selection,
    selection_diversity_weight: f64,
    selection_pool_factor: usize,
    reduce_cover: bool,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>> {
    let init_input = &session.init_input;
    let mut init_metadata = session.init_metadata.clone();
    let corpus = session.state.corpus();
    let mut passed_cases = Vec::new();
    for id in corpus.ids().collect::<Vec<_>>() {
        let testcase = corpus.get(id)?.borrow();
        let metadata = case_metadata(&testcase)?;
        let passed = metadata.is_passed;

        if !passed {
            continue;
        }

        if metadata.state_trackers.len() == 0 {
            continue;
        }

        let cover_distance = coverage_distance(&init_metadata.covers, &metadata.covers);
        let state_distance =
            state_trackers_distance(&init_metadata.state_trackers, &metadata.state_trackers);

        log::debug!(
            "Corpus testcase {id}: cover_distance {}, state_distance {}",
            cover_distance,
            state_distance
        );

        let input = testcase
            .input()
            .as_ref()
            .ok_or(format!("Corpus testcase {id} has no input"))?;

        if !cover_distance.is_finite() || !state_distance.is_finite() {
            return Err(format!(
                "Non-finite fail distance for corpus case {id}: coverage={cover_distance}, state={state_distance}"
            )
            .into());
        }

        passed_cases.push(PassedCase {
            id: usize::from(id),
            input: input.clone(),
            metadata,
            fail_cover_distance: cover_distance,
            fail_state_distance: state_distance,
            fail_distance: 0.0,
        });
    }

    let fail_cover_distances = passed_cases
        .iter()
        .map(|case| case.fail_cover_distance)
        .collect::<Vec<_>>();
    let fail_state_distances = passed_cases
        .iter()
        .map(|case| case.fail_state_distance)
        .collect::<Vec<_>>();
    let (cover_distances_trans, state_distances_trans) = (
        quantile_transform(&fail_cover_distances),
        quantile_transform(&fail_state_distances),
    );

    for ((case, cover), state) in passed_cases
        .iter_mut()
        .zip(cover_distances_trans)
        .zip(state_distances_trans)
    {
        case.fail_distance = combine_raw_distance(cover, state, cover_weight);
    }

    let limit = usize::min(top_pass, passed_cases.len());
    let decisions = match selection {
        Selection::Random => {
            let mut indices = (0..passed_cases.len()).collect::<Vec<_>>();
            let mut rng = rand::thread_rng();
            indices.shuffle(&mut rng);
            indices
                .into_iter()
                .take(limit)
                .map(|index| SelectionDecision {
                    index,
                    min_pass_distance: 0.0,
                    score: 1.0 - passed_cases[index].fail_distance,
                })
                .collect()
        }
        Selection::Sort => {
            let mut indices = (0..passed_cases.len()).collect::<Vec<_>>();
            indices.sort_by(|&a, &b| {
                passed_cases[a]
                    .fail_distance
                    .total_cmp(&passed_cases[b].fail_distance)
                    .then_with(|| passed_cases[a].id.cmp(&passed_cases[b].id))
            });
            indices
                .into_iter()
                .take(limit)
                .map(|index| SelectionDecision {
                    index,
                    min_pass_distance: 0.0,
                    score: 1.0 - passed_cases[index].fail_distance,
                })
                .collect()
        }
        Selection::Diverse => select_diverse_passed_cases(
            &passed_cases,
            limit,
            cover_weight,
            selection_diversity_weight,
            selection_pool_factor,
        )?,
    };

    log::info!(
        "Found {} passed testcases with unique coverage, selecting {} cases by {:?}.",
        passed_cases.len(),
        limit,
        selection
    );

    let pass_selection_report =
        calculate_pass_selection_report(&init_metadata, &passed_cases, &decisions);
    log::info!(
        "Selected pass RWMFC={:.6}%, reachability={:.6}% ({}/{} fail points reachable), selected_covered_reachable_fail_points={}",
        pass_selection_report.final_rwmfc_percent,
        pass_selection_report.reachability_percent,
        pass_selection_report.reachable_fail_points,
        pass_selection_report.total_fail_points,
        pass_selection_report.selected_covered_reachable_fail_points,
    );
    for case in &pass_selection_report.cases {
        log::info!(
            "Pass selection metric: rank={}, corpus_id={}, distance={:.6}, marginal_rwmfc={:.6}%, cumulative_rwmfc={:.6}%, new_reachable_fail_points={}",
            case.rank,
            case.corpus_id,
            case.distance,
            case.marginal_rwmfc_percent,
            case.cumulative_rwmfc_percent,
            case.new_reachable_fail_points,
        );
    }
    if let Some(output_dir) = &output {
        write_pass_selection_metrics(output_dir, &pass_selection_report)?;
        write_pass_selection_summary(output_dir, &pass_selection_report)?;
        write_fail_point_difficulties(output_dir, &pass_selection_report, cover_point_name)?;
    }

    let mut passed_cases = passed_cases.into_iter().map(Some).collect::<Vec<_>>();
    let mut top_passed_cases = decisions
        .into_iter()
        .map(|decision| {
            (
                passed_cases[decision.index]
                    .take()
                    .expect("Pass case selected more than once"),
                decision,
            )
        })
        .collect::<Vec<_>>();

    if reduce_cover {
        let init_pc_tracker = init_metadata.state_trackers.pc_tracker.clone();
        reduce_init_case_coverage(init_input, &mut init_metadata);
        for (case, _) in top_passed_cases.iter_mut() {
            reduce_pass_case_coverage(&case.input, &init_pc_tracker, &mut case.metadata);
        }
    }

    for (rank, (case, decision)) in top_passed_cases.iter().enumerate() {
        if matches!(selection, Selection::Diverse) {
            log::info!(
                "Top {} passed testcase: corpus_id={}, fail_distance={:.6}, min_pass_distance={:.6}, score={:.6}",
                rank + 1,
                case.id,
                case.fail_distance,
                decision.min_pass_distance,
                decision.score
            );
        } else {
            log::info!(
                "Top {} passed testcase: corpus_id={}, distance={:.6}",
                rank + 1,
                case.id,
                case.fail_distance
            );
        }

        if let Some(output_dir) = &output {
            let filename = format!("rank_{:04}_id_{}", rank + 1, case.id,);
            store_testcase(
                &case.input,
                save_intermediate.then(|| &case.metadata),
                output_dir,
                Some(&filename),
            )?;
        }
    }

    let mut case_coverages: Vec<CaseMetadata> = top_passed_cases
        .into_iter()
        .map(|(case, _)| case.metadata)
        .collect();
    case_coverages.push(init_metadata);
    Ok(case_coverages)
}
