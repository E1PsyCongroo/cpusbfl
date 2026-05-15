use std::{fs, path::PathBuf, time::Duration};

use dtw_rs::Distance;
use libafl::{
    StdFuzzer, mutators::scheduled::SingleChoiceScheduledMutator as StdScheduledMutator,
    prelude::*, schedulers::QueueScheduler, state::StdState,
};
use libafl_bolts::tuples::Append;
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list};

use crate::coverage::*;
use crate::feedback::{coverages_feedback::*, statetracker_feedback::*};
use crate::harness::{self, SIM_ARGS};
use crate::monitor;
use crate::mutator::lastinst_mutator::*;
use crate::observer::{coverages_observer::*, statetracker_observer::*};
use crate::pc_trace::*;
use crate::similarity::*;
use crate::state_tracker::*;

fn load_initial_case(corpus_input: &String) -> BytesInput {
    let path = PathBuf::from(corpus_input);
    let input_path = if path.is_file() {
        path.clone()
    } else if path.is_dir() {
        let mut entries = fs::read_dir(&path)
            .unwrap_or_else(|err| panic!("Failed to read corpus_input {path:?}: {err}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|entry_path| entry_path.is_file())
            .collect::<Vec<_>>();
        entries.sort();
        entries
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("No testcase found in corpus_input directory {path:?}"))
    } else {
        panic!("corpus_input {path:?} is neither a file nor a directory")
    };

    let bytes = fs::read(&input_path)
        .unwrap_or_else(|err| panic!("Failed to read initial fault case {input_path:?}: {err}"));

    BytesInput::new(bytes)
}

pub(crate) struct CaseMetadata {
    pub covers: Coverages,
    pub state_track: StateTracker,
    pub is_passed: bool,
}

fn emit_top_passed_testcases(
    state: &StdState<
        InMemoryCorpus<ValueInput<Vec<u8>>>,
        ValueInput<Vec<u8>>,
        StdRand,
        OnDiskCorpus<ValueInput<Vec<u8>>>,
    >,
    init_metadata: CaseMetadata,
    top_n: u64,
    corpus_output: Option<String>,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>> {
    let corpus = state.corpus();
    let mut passed_cases = Vec::new();

    for id in corpus.ids() {
        let testcase = corpus.get(id)?.borrow();

        let cover = testcase.metadata::<CoveragesMetadata>()?;
        let track = testcase.metadata::<StateTrackerMetadata>()?;
        assert_eq!(cover.is_passed, track.is_passed);

        if !cover.is_passed || !track.is_passed {
            continue;
        }

        let metadata = CaseMetadata {
            covers: cover.covers.clone(),
            state_track: track.track.clone(),
            is_passed: cover.is_passed && track.is_passed,
        };

        let cover_distance = init_metadata
            .covers
            .names()
            .iter()
            .map(|cov_name| {
                let init_counts = init_metadata.covers.get(cov_name).covered_counts();
                let metadata_counts = metadata.covers.get(cov_name).covered_counts();
                let dis = euclidean_distance(&init_counts, &metadata_counts)
                    / (init_metadata.covers.get(cov_name).len() as f64).sqrt();
                dis
            })
            .sum::<f64>()
            / init_metadata.covers.names().len() as f64;

        let state_distantce =
            fastdtw_distance(&init_metadata.state_track, &metadata.state_track, 10)?
                / init_metadata.state_track.state_size() as f64;

        println!(
            "cover_distance: {}, state_distance: {}",
            cover_distance, state_distantce
        );

        let distance = cover_distance + state_distantce;

        let input = testcase
            .input()
            .as_ref()
            .ok_or(Error::illegal_state(format!(
                "Corpus testcase {id} has no input"
            )))?;

        passed_cases.push((usize::from(id), input.clone(), metadata, distance));
    }

    passed_cases.sort_by(|a, b| {
        a.3.partial_cmp(&b.3)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let limit = usize::min(top_n as usize, passed_cases.len());
    println!(
        "Found {} passed testcases with unique coverage, selecting top {}.",
        passed_cases.len(),
        limit
    );

    // for (id, input, metadata, distance) in passed_cases.iter() {
    //     let filename = format!("id_{:04}_dst_{:.6}", id, distance);
    //     monitor::store_testcase(input, Some(metadata), &"debug2".to_string(), Some(filename));
    // }

    let top_passed_cases: Vec<_> = passed_cases.into_iter().take(limit).collect();
    for (rank, (id, input, metadata, distance)) in top_passed_cases.iter().enumerate() {
        println!(
            "Top {} passed testcase: corpus_id={}, distance={:.6}",
            rank + 1,
            id,
            distance
        );

        if let Some(output_dir) = &corpus_output {
            let filename = format!("rank_{:04}_id_{}_dst_{:.6}", rank + 1, id, distance);
            monitor::store_testcase(input, Some(metadata), output_dir, Some(filename));
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
    max_iters: u64,
    max_run_timeout: u64,
    top_n: u64,
    corpus_input: String,
    corpus_output: Option<String>,
) -> Result<Vec<CaseMetadata>, Box<dyn std::error::Error>> {
    // Scheduler, Feedback, Objective
    let scheduler = QueueScheduler::new();

    let coverages_observer = unsafe { CoveragesObserver::from_raw("coverages", &coverages()) };
    let statetracker_observer =
        unsafe { StateTrackerObserver::from_raw("state_tracker", &tracker("ArchIntRegState")) };

    let mut feedback = feedback_or!(
        CoveragesFeedback::new(&coverages_observer),
        StateTrackerFeedback::new(&statetracker_observer)
    );
    let mut objective = ConstFeedback::new(false);

    // State, Manager
    let mut state = StdState::new(
        StdRand::with_seed(current_nanos()),
        InMemoryCorpus::new(),
        OnDiskCorpus::new(PathBuf::from("./crashes")).unwrap(),
        &mut feedback,
        &mut objective,
    )
    .unwrap();
    let monitor = SimpleMonitor::new(|s| {
        println!("{}", s);
    });
    let mut mgr = SimpleEventManager::new(monitor);

    // Fuzzer, Executor
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);
    let mut binding = harness::fuzz_harness;
    let mut executor = InProcessExecutor::with_timeout(
        &mut binding,
        tuple_list!(coverages_observer, statetracker_observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        Duration::from_secs(max_run_timeout),
    )
    .unwrap();

    // Initial Case
    let init_bytes = load_initial_case(&corpus_input);
    let init_metadata;
    if let (ExecuteInputResult::Corpus, Some(init_corpus_id)) =
        fuzzer.evaluate_input(&mut state, &mut executor, &mut mgr, &init_bytes)?
    {
        let init_testcase = state.corpus_mut().get(init_corpus_id)?.borrow_mut();

        let init_cover = init_testcase.metadata::<CoveragesMetadata>()?;
        if init_cover.is_passed {
            return Err(Box::new(Error::illegal_argument(format!(
                "Initial case from {corpus_input:?} did not crash"
            ))));
        }

        let init_state = init_testcase.metadata::<StateTrackerMetadata>()?;
        if init_cover.is_passed {
            return Err(Box::new(Error::illegal_argument(format!(
                "Initial case from {corpus_input:?} did not crash"
            ))));
        }

        init_metadata = CaseMetadata {
            covers: init_cover.covers.to_owned(),
            state_track: init_state.track.to_owned(),
            is_passed: false,
        };
    } else {
        return Err(Box::new(Error::illegal_argument(format!(
            "Initial case from {corpus_input:?} was not accepted into the main corpus by feedback"
        ))));
    }

    pc_trace_update_stats();

    let max_inst = init_metadata.state_track.len();
    let last_pc = pc_trace().as_slice()[0];
    SIM_ARGS
        .get()
        .unwrap()
        .lock()
        .expect("poisoned mutex")
        .push(format!("-I {max_inst}"));

    // Fuzzing Loop
    let mutator = StdScheduledMutator::new(tuple_list!(LastInstMutator::new(last_pc)?));
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    fuzzer.fuzz_loop_for(&mut stages, &mut executor, &mut state, &mut mgr, max_iters)?;

    // for cover_name in cover_names() {
    //     println!("init_case cover points of {cover_name}:");
    //     for (point, count) in init_metadata
    //         .covers
    //         .get(&cover_name)
    //         .covered_counts()
    //         .into_iter()
    //         .enumerate()
    //     {
    //         println!(
    //             "cover point: \"{}\"({})",
    //             cover_point_name(&cover_name, point),
    //             count
    //         );
    //     }
    // }

    emit_top_passed_testcases(&state, init_metadata, top_n, corpus_output)
}
