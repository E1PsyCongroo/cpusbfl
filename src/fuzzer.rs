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
}

impl Default for Selection {
    fn default() -> Self {
        Self::Sort
    }
}

fn emit_top_passed_testcases(
    init_input: &BytesInput,
    output: &Option<String>,

    cover_weight: f64,
    top_pass: u64,
    selection: Selection,
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

        passed_cases.push((
            usize::from(id),
            input.clone(),
            metadata,
            cover_distance,
            state_distance,
        ));
    }

    let cover_distances_trans = quantile_transform(
        &passed_cases
            .iter()
            .map(|&(_, _, _, cover_distance, _)| cover_distance)
            .collect::<Vec<_>>(),
    );
    let state_distances_trans = quantile_transform(
        &passed_cases
            .iter()
            .map(|&(_, _, _, _, state_distance)| state_distance)
            .collect::<Vec<_>>(),
    );

    let mut passed_cases = passed_cases
        .into_iter()
        .zip(cover_distances_trans)
        .zip(state_distances_trans)
        .map(|(((id, input, metadata, _, _), cover), state)| {
            (
                id,
                input,
                metadata,
                cover_weight * cover + (1.0 - cover_weight) * state,
            )
        })
        .collect::<Vec<_>>();

    match selection {
        Selection::Random => {
            let mut rng = rand::thread_rng();
            passed_cases.shuffle(&mut rng);
        }
        Selection::Sort => {
            passed_cases.sort_by(|a, b| {
                a.3.partial_cmp(&b.3)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
        }
    }

    let limit = usize::min(top_pass as usize, passed_cases.len());
    log::info!(
        "Found {} passed testcases with unique coverage, selecting {} cases by {:?}.",
        passed_cases.len(),
        limit,
        selection
    );

    let mut top_passed_cases: Vec<_> = passed_cases.into_iter().take(limit).collect();

    if reduce_cover {
        let init_pc_tracker = init_metadata.state_trackers.pc_tracker.clone();
        reduce_init_case_coverage(init_input, &mut init_metadata);
        for (_, input, metadata, _) in top_passed_cases.iter_mut() {
            reduce_pass_case_coverage(input, &init_pc_tracker, metadata);
        }
    }

    for (rank, (id, input, metadata, distance)) in top_passed_cases.iter().enumerate() {
        log::info!(
            "Top {} passed testcase: corpus_id={}, distance={:.6}",
            rank + 1,
            id,
            distance
        );

        if let Some(output_dir) = &output {
            let filename = format!("rank_{:04}_id_{}_dst_{:.6}", rank + 1, id, distance);
            store_testcase(
                input,
                save_trace.then(|| metadata),
                output_dir,
                Some(&filename),
            )?;
        }
    }

    let mut case_coverages: Vec<CaseMetadata> = top_passed_cases
        .into_iter()
        .map(|(_, _, meta, _)| meta)
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
        reduce_cover,
        save_trace,
        state,
        init_metadata,
    )
}
