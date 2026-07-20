use std::collections::HashSet;
use std::{any::type_name, io::Write};

use clap::ValueEnum;
use libafl::{StdFuzzer, prelude::*, state::StdState};
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list};
use rand::seq::SliceRandom;

use crate::coverage::*;
use crate::feedback::*;
use crate::harness::*;
use crate::mutator::*;
use crate::observer::*;
use crate::reduce::*;
use crate::similarity::*;
use crate::state_tracker::*;
use crate::utils::*;

pub(crate) struct CaseMetadata {
    pub covers: Coverages,
    pub state_trackers: StateTrackers,
    pub is_passed: bool,
    pub mutated_pcs: Option<HashSet<u64>>,
}

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

fn emit_top_passed_testcases(
    init_input: &BytesInput,
    output: &Option<String>,

    cover_weight: f64,
    top_pass: u64,
    selection: Selection,
    selection_diversity_weight: f64,
    selection_pool_factor: usize,
    reduce_cover: bool,
    save_trace: bool,

    mut state: StdState<
        InMemoryCorpus<BytesInput>,
        BytesInput,
        StdRand,
        InMemoryCorpus<BytesInput>,
    >,
    mut init_metadata: CaseMetadata,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>> {
    let corpus = state.corpus_mut();
    let mut passed_cases = Vec::new();
    for id in corpus.ids().collect::<Vec<_>>() {
        let mut testcase = corpus.remove(id)?;

        let passed = testcase
            .remove_metadata::<PassedMetadata>()
            .ok_or_else(|| format!("{} not found", type_name::<PassedMetadata>()))?
            .is_passed;

        if !passed {
            continue;
        }

        let covers = testcase
            .remove_metadata::<CoveragesMetadata>()
            .ok_or_else(|| format!("{} not found", type_name::<CoveragesMetadata>()))?
            .covers;
        let trackers = testcase
            .remove_metadata::<StateTrackersMetadata>()
            .ok_or_else(|| format!("{} not found", type_name::<StateTrackersMetadata>()))?
            .trackers;

        if trackers.len() == 0 {
            continue;
        }

        let mutated_pcs = testcase
            .remove_metadata::<MutationMetadata>()
            .map(|m| m.mutated_pcs);

        let metadata = CaseMetadata {
            covers: covers,
            state_trackers: trackers,
            is_passed: passed,
            mutated_pcs: mutated_pcs,
        };

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

    let limit = usize::min(top_pass as usize, passed_cases.len());
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
            let filename = format!(
                "rank_{:04}_id_{}_dst_{:.6}",
                rank + 1,
                case.id,
                case.fail_distance
            );
            store_testcase(
                &case.input,
                save_trace.then(|| &case.metadata),
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

pub(crate) fn run_fuzzer<SC, M, MF>(
    init_case: &BytesInput,
    output: &Option<String>,
    max_iters: u64,
    max_run_timeout: u64,
    tracker_window_size: u64,

    cover_weight: f64,
    top_pass: u64,
    selection: Selection,
    selection_diversity_weight: f64,
    selection_pool_factor: usize,
    reduce_cover: bool,
    save_trace: bool,

    scheduler: SC,
    mutator_factory: MF,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>>
where
    SC: Scheduler<
            BytesInput,
            StdState<InMemoryCorpus<BytesInput>, BytesInput, StdRand, InMemoryCorpus<BytesInput>>,
        >,
    M: Mutator<
            BytesInput,
            StdState<InMemoryCorpus<BytesInput>, BytesInput, StdRand, InMemoryCorpus<BytesInput>>,
        >,
    MF: FnOnce(&CaseMetadata) -> Result<M, Box<dyn std::error::Error>>,
{
    // Scheduler, Feedback, Objective
    let coverages_observer = unsafe { CoveragesObserver::from_raw("coverages", &coverages()) };
    let statetrackers_observer = unsafe {
        StateTrackersObserver::from_raw("state_trackers", &trackers(), tracker_window_size)
    };

    let mut feedback = feedback_and_fast!(
        PassedFeedback::new(),
        feedback_or!(
            CoveragesFeedback::new(&coverages_observer),
            StateTrackersFeedback::new(&statetrackers_observer)
        )
    );
    let mut objective = ConstFeedback::new(false);

    // State, Manager
    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        InMemoryCorpus::new(),
        InMemoryCorpus::new(),
        &mut feedback,
        &mut objective,
    )
    .unwrap();
    let monitor = SimpleMonitor::new(|s| {
        log::info!("{s}");
    });
    let mut mgr = SimpleEventManager::new(monitor);

    // Fuzzer, Executor
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);
    let mut binding = fuzz_harness;
    let mut executor = InProcessExecutor::with_timeout(
        &mut binding,
        tuple_list!(coverages_observer, statetrackers_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        std::time::Duration::from_secs(max_run_timeout),
    )
    .unwrap();

    // Initial Case

    let init_metadata;
    if let (ExecuteInputResult::Corpus, Some(init_corpus_id)) =
        fuzzer.evaluate_input(&mut state, &mut executor, &mut mgr, init_case)?
    {
        let init_testcase = state.corpus_mut().get(init_corpus_id)?.borrow_mut();

        let init_passed = init_testcase.metadata::<PassedMetadata>()?.is_passed;
        if init_passed {
            return Err("Initial case did not crash".into());
        }

        let init_cover = init_testcase.metadata::<CoveragesMetadata>()?;
        let init_state = init_testcase.metadata::<StateTrackersMetadata>()?;

        init_metadata = CaseMetadata {
            covers: init_cover.covers.to_owned(),
            state_trackers: init_state.trackers.to_owned(),
            is_passed: init_passed,
            mutated_pcs: None,
        };
    } else {
        return Err("Initial case was not accepted into the main corpus by feedback".into());
    }

    if let Some(output_dir) = output.as_ref() {
        store_testcase(
            init_case,
            save_trace.then(|| &init_metadata),
            output_dir,
            Some("init_case"),
        )?;
    }

    let max_inst = trackers().len();
    SIM_ARGS
        .get()
        .unwrap()
        .lock()
        .expect("SIM_ARGS poisoned mutex")
        .extend(vec!["-I".to_string(), max_inst.to_string()].into_iter());

    // Build mutators after evaluating the initial input, since the guided
    // mutators need its PC trace.
    let mutator = mutator_factory(&init_metadata)?;

    // Fuzzing Loop
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    let fuzzing_start_time = process_cpu_time_now()?;
    fuzzer.fuzz_loop_for(&mut stages, &mut executor, &mut state, &mut mgr, max_iters)?;
    let fuzzing_end_time = process_cpu_time_now()?;
    let fuzzing_elapsed = fuzzing_end_time
        .checked_sub(fuzzing_start_time)
        .unwrap_or_default();

    log::info!("Fuzzing process CPU time = {fuzzing_elapsed:?}");

    if let Some(output_dir) = output.as_ref() {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(std::path::PathBuf::from(format!("{output_dir}/fuzzing_time.txt")).as_path())?
            .write_fmt(format_args!("{fuzzing_elapsed:?}"))?;
    }

    for cover_name in cover_names() {
        log::trace!("init_case cover points of {cover_name}:");
        for (point, count) in init_metadata
            .covers
            .covered_counts(&cover_name)
            .into_iter()
            .enumerate()
        {
            log::trace!(
                "cover point: \"{}\"({})",
                cover_point_name(&cover_name, point),
                count
            );
        }
    }

    emit_top_passed_testcases(
        init_case,
        output,
        cover_weight,
        top_pass,
        selection,
        selection_diversity_weight,
        selection_pool_factor,
        reduce_cover,
        save_trace,
        state,
        init_metadata,
    )
}
