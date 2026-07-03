use std::borrow::Cow;
use std::{any::type_name, io::Write};

use libafl::{StdFuzzer, prelude::*, schedulers::QueueScheduler, state::StdState};
use libafl_bolts::{Named, current_nanos, rands::StdRand, tuples::tuple_list};

use crate::coverage::*;
use crate::feedback::{coverages_feedback::*, passed_feedback::*, statetrackers_feedback::*};
use crate::harness::{SIM_ARGS, fuzz_harness};
use crate::mutator::{
    elf_scheduled::ELFHavocScheduledMutator, lastwindow_mutator::LastWindowMutator,
};
use crate::observer::{coverages_observer::*, statetrackers_observer::*};
use crate::similarity::*;
use crate::state_tracker::*;
use crate::utils::*;

#[derive(Debug)]
enum SelectedMutator<MT> {
    Elf(ELFHavocScheduledMutator<MT>),
    Last(LastWindowMutator),
}

impl<MT> Named for SelectedMutator<MT> {
    fn name(&self) -> &Cow<'static, str> {
        match self {
            SelectedMutator::Elf(m) => m.name(),
            SelectedMutator::Last(m) => m.name(),
        }
    }
}

impl<I, MT, S> Mutator<I, S> for SelectedMutator<MT>
where
    I: HasMutatorBytes,
    MT: MutatorsTuple<BytesInput, S>,
    S: HasRand,
    LastWindowMutator: Mutator<I, S>,
{
    #[inline]
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        match self {
            SelectedMutator::Elf(m) => m.mutate(state, input),
            SelectedMutator::Last(m) => m.mutate(state, input),
        }
    }

    #[inline]
    fn post_exec(&mut self, state: &mut S, new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        match self {
            SelectedMutator::Elf(m) => {
                <ELFHavocScheduledMutator<MT> as Mutator<I, S>>::post_exec(m, state, new_corpus_id)
            }

            SelectedMutator::Last(m) => {
                <LastWindowMutator as Mutator<I, S>>::post_exec(m, state, new_corpus_id)
            }
        }
    }
}

pub(crate) struct CaseMetadata {
    pub covers: Coverages,
    pub state_trackers: StateTrackers,
    pub is_passed: bool,
}

fn emit_top_passed_testcases(
    mut state: StdState<
        InMemoryCorpus<ValueInput<Vec<u8>>>,
        ValueInput<Vec<u8>>,
        StdRand,
        InMemoryCorpus<ValueInput<Vec<u8>>>,
    >,
    init_metadata: CaseMetadata,
    top_n: u64,
    save_trace: bool,
    output: &Option<String>,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>> {
    let corpus = state.corpus_mut();
    let mut passed_cases = Vec::new();
    let init_metadata_coverd_counts = init_metadata
        .covers
        .names()
        .into_iter()
        .map(|cover_name| {
            let cover = init_metadata.covers.get(&cover_name);
            (cover_name, cover.len(), cover.covered_counts())
        })
        .collect::<Vec<_>>();
    let init_metadata_core_state = init_metadata
        .state_trackers
        .arch_int_reg_tracker
        .iter()
        .zip(init_metadata.state_trackers.csr_tracker.iter())
        .map(|(arch, csr)| CoreStateRef {
            arch_int_reg_state: arch,
            csr_state: csr,
        })
        .collect::<Vec<_>>();

    for id in corpus.ids().collect::<Vec<_>>() {
        let mut testcase = corpus.remove(id)?;

        let passed = testcase
            .remove_metadata::<PassedMetadata>()
            .ok_or_else(|| format!("{} not found", type_name::<PassedMetadata>()))?
            .is_passed;

        if !passed {
            continue;
        }

        let cover = *testcase
            .remove_metadata::<CoveragesMetadata>()
            .ok_or_else(|| format!("{} not found", type_name::<CoveragesMetadata>()))?;
        let tracker = *testcase
            .remove_metadata::<StateTrackersMetadata>()
            .ok_or_else(|| format!("{} not found", type_name::<StateTrackersMetadata>()))?;

        let metadata = CaseMetadata {
            covers: cover.covers,
            state_trackers: tracker.trackers,
            is_passed: passed,
        };

        let cover_distance = init_metadata_coverd_counts
            .iter()
            .map(|(cov_name, cov_len, init_counts)| {
                let metadata_counts = metadata.covers.get(cov_name).covered_counts();
                let dis =
                    euclidean_distance(&init_counts, &metadata_counts) / (*cov_len as f64).sqrt();
                dis
            })
            .sum::<f64>()
            / init_metadata_coverd_counts.len() as f64;

        let state_distance = fastdtw_distance(
            &init_metadata_core_state,
            &metadata
                .state_trackers
                .arch_int_reg_tracker
                .iter()
                .zip(metadata.state_trackers.csr_tracker.iter())
                .map(|(arch, csr)| CoreStateRef {
                    arch_int_reg_state: arch,
                    csr_state: csr,
                })
                .collect::<Vec<_>>(),
            10,
        );

        log::info!(
            "Corpus testcase {id}: cover_distance {}, state_distance {}",
            cover_distance,
            state_distance
        );

        let distance = cover_distance + state_distance;

        let input = testcase
            .input()
            .as_ref()
            .ok_or(format!("Corpus testcase {id} has no input"))?;

        passed_cases.push((usize::from(id), input.clone(), metadata, distance));
    }

    passed_cases.sort_by(|a, b| {
        a.3.partial_cmp(&b.3)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let limit = usize::min(top_n as usize, passed_cases.len());
    log::info!(
        "Found {} passed testcases with unique coverage, selecting top {}.",
        passed_cases.len(),
        limit
    );

    let top_passed_cases: Vec<_> = passed_cases.into_iter().take(limit).collect();
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

pub(crate) fn run_fuzzer(
    base_mutator: bool,
    max_iters: u64,
    max_run_timeout: u64,
    tracker_window_size: u64,
    mutator_window_size: u64,
    top_n: u64,
    save_trace: bool,
    init_case: &BytesInput,
    output: &Option<String>,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>> {
    // Scheduler, Feedback, Objective
    let scheduler = QueueScheduler::new();

    let coverages_observer = unsafe { CoveragesObserver::from_raw("coverages", &coverages()) };
    let statetrackers_observer = unsafe {
        StateTrackersObserver::from_raw("state_trackers", &trackers(), tracker_window_size)
    };

    let mut feedback = feedback_and!(
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

    // Fuzzing Loop
    let mutator = if base_mutator {
        SelectedMutator::Elf(ELFHavocScheduledMutator::new(havoc_mutations(), init_case)?)
    } else {
        SelectedMutator::Last(LastWindowMutator::new(
            init_case,
            &init_metadata.state_trackers.pc_tracker,
            mutator_window_size,
        )?)
    };
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    let fuzzing_start_time = process_cpu_time_now()?;
    fuzzer.fuzz_loop_for(&mut stages, &mut executor, &mut state, &mut mgr, max_iters)?;
    let fuzzing_end_time = process_cpu_time_now()?;
    let fuzzing_elapsed = fuzzing_end_time
        .checked_sub(fuzzing_start_time)
        .unwrap_or_default();

    log::info!("Fuzzing process CPU time = {fuzzing_elapsed:?}");

    if let Some(output_dir) = output.as_ref() {
        let mut fuzzing_time_fs = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(std::path::PathBuf::from(format!("{output_dir}/fuzzing_time.txt")).as_path())?;
        writeln!(&mut fuzzing_time_fs, "{fuzzing_elapsed:?}")?;
    }

    for cover_name in cover_names() {
        log::trace!("init_case cover points of {cover_name}:");
        for (point, count) in init_metadata
            .covers
            .get(&cover_name)
            .covered_counts()
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

    emit_top_passed_testcases(state, init_metadata, top_n, save_trace, output)
}
