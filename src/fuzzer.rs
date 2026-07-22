use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use libafl::{StdFuzzer, prelude::*, state::StdState};
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list};

use crate::coverage::*;
use crate::feedback::*;
use crate::harness::*;
use crate::mutator::*;
use crate::observer::*;
use crate::state_tracker::*;
use crate::utils::*;

#[derive(Clone)]
pub(crate) struct CaseMetadata {
    pub covers: Coverages,
    pub state_trackers: StateTrackers,
    pub is_passed: bool,
    pub mutated_pcs: Option<HashSet<u64>>,
}

pub(crate) type FuzzerState =
    StdState<InMemoryCorpus<BytesInput>, BytesInput, StdRand, InMemoryCorpus<BytesInput>>;

pub(crate) struct FuzzSession {
    pub(crate) state: FuzzerState,
    pub(crate) initial_corpus_id: CorpusId,
    pub(crate) init_input: BytesInput,
    pub(crate) init_metadata: CaseMetadata,
    pub(crate) completed_iters: u64,
}

impl FuzzSession {
    pub(crate) fn from_restored_state(
        state: FuzzerState,
        initial_corpus_id: CorpusId,
        completed_iters: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let (init_input, init_metadata) = {
            let testcase = state.corpus().get(initial_corpus_id)?.borrow();
            let input = testcase
                .input()
                .as_ref()
                .ok_or(format!(
                    "Initial corpus testcase {initial_corpus_id} has no input"
                ))?
                .clone();
            (input, case_metadata(&testcase)?)
        };

        if init_metadata.is_passed {
            return Err("Initial testcase in the corpus checkpoint did not crash".into());
        }

        Ok(Self {
            state,
            initial_corpus_id,
            init_input,
            init_metadata,
            completed_iters,
        })
    }
}

pub(crate) fn case_metadata(
    testcase: &Testcase<BytesInput>,
) -> Result<CaseMetadata, Box<dyn std::error::Error>> {
    let is_passed = testcase.metadata::<PassedMetadata>()?.is_passed;
    let covers = testcase.metadata::<CoveragesMetadata>()?.covers.clone();
    let state_trackers = testcase
        .metadata::<StateTrackersMetadata>()?
        .trackers
        .clone();
    let mutated_pcs = testcase
        .metadata::<MutationMetadata>()
        .ok()
        .map(|metadata| metadata.mutated_pcs.clone());

    Ok(CaseMetadata {
        covers,
        state_trackers,
        is_passed,
        mutated_pcs,
    })
}

pub(crate) fn run_fuzzer<SC, M, SF, MF, CF>(
    init_case: Option<&BytesInput>,
    resume: Option<FuzzSession>,
    output: Option<&PathBuf>,
    max_iters: u64,
    max_run_timeout: u64,
    tracker_window_size: usize,
    save_intermediate: bool,
    checkpoint_interval: Option<u64>,
    scheduler_factory: SF,
    mutator_factory: MF,
    mut checkpoint_callback: CF,
) -> Result<FuzzSession, Box<dyn std::error::Error>>
where
    SC: Scheduler<BytesInput, FuzzerState>,
    M: Mutator<BytesInput, FuzzerState>,
    SF: FnOnce() -> Result<SC, Box<dyn std::error::Error>>,
    MF: FnOnce(
        &BytesInput,
        &CaseMetadata,
        &mut FuzzerState,
    ) -> Result<M, Box<dyn std::error::Error>>,
    CF: FnMut(&FuzzSession) -> Result<(), Box<dyn std::error::Error>>,
{
    if max_iters == 0 {
        return Err("max_iters must be greater than 0".into());
    }
    if checkpoint_interval == Some(0) {
        return Err("checkpoint_interval must be greater than 0".into());
    }
    match (init_case.is_some(), resume.is_some()) {
        (true, true) => return Err("init_case and resume are mutually exclusive".into()),
        (false, false) => return Err("Either init_case or resume must be provided".into()),
        _ => {}
    }

    // Scheduler, Feedback, Objective
    let scheduler = scheduler_factory()?;
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

    let (mut state, restored_session) = match resume {
        Some(session) => {
            let FuzzSession {
                state,
                initial_corpus_id,
                init_input,
                init_metadata,
                completed_iters,
            } = session;
            (
                state,
                Some((
                    initial_corpus_id,
                    init_input,
                    init_metadata,
                    completed_iters,
                )),
            )
        }
        None => (
            StdState::new(
                StdRand::with_seed(current_nanos()),
                InMemoryCorpus::new(),
                InMemoryCorpus::new(),
                &mut feedback,
                &mut objective,
            )?,
            None,
        ),
    };

    // State, Manager
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

    let mut session = if let Some((initial_corpus_id, init_input, init_metadata, completed_iters)) =
        restored_session
    {
        FuzzSession {
            state,
            initial_corpus_id,
            init_input,
            init_metadata,
            completed_iters,
        }
    } else {
        let init_case = init_case.ok_or("corpus_input is required when not resuming a corpus")?;
        let (initial_corpus_id, init_metadata) =
            if let (ExecuteInputResult::Corpus, Some(initial_corpus_id)) =
                fuzzer.evaluate_input(&mut state, &mut executor, &mut mgr, init_case)?
            {
                let init_metadata = {
                    let testcase = state.corpus().get(initial_corpus_id)?.borrow();
                    case_metadata(&testcase)?
                };
                if init_metadata.is_passed {
                    return Err("Initial case did not crash".into());
                }
                (initial_corpus_id, init_metadata)
            } else {
                return Err(
                    "Initial case was not accepted into the main corpus by feedback".into(),
                );
            };

        FuzzSession {
            state,
            initial_corpus_id,
            init_input: init_case.clone(),
            init_metadata,
            completed_iters: 0,
        }
    };

    if let Some(output_dir) = output {
        store_testcase(
            &session.init_input,
            save_intermediate.then(|| &session.init_metadata),
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
    let mutator = mutator_factory(
        &session.init_input,
        &session.init_metadata,
        &mut session.state,
    )?;

    // Fuzzing Loop
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    if let Some(interval) = checkpoint_interval {
        for _ in 0..max_iters {
            fuzzer.fuzz_one(&mut stages, &mut executor, &mut session.state, &mut mgr)?;
            session.completed_iters = session
                .completed_iters
                .checked_add(1)
                .ok_or("completed_iters overflow")?;
            if session.completed_iters % interval == 0 {
                checkpoint_callback(&session)?;
            }
        }
    } else {
        fuzzer.fuzz_loop_for(
            &mut stages,
            &mut executor,
            &mut session.state,
            &mut mgr,
            max_iters,
        )?;
        session.completed_iters = session
            .completed_iters
            .checked_add(max_iters)
            .ok_or("completed_iters overflow")?;
    }

    for cover_name in cover_names() {
        log::trace!("init_case cover points of {cover_name}:");
        for (point, count) in session
            .init_metadata
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

    Ok(session)
}
