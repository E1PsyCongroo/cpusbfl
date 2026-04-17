use core::slice;
/**
 * Copyright (c) 2023 Institute of Computing Technology, Chinese Academy of Sciences
 * xfuzz is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL v2.
 * You may obtain a copy of Mulan PSL v2 at:
 *          http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY KIND,
 * EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT,
 * MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
use std::{collections::HashMap, fs, path::PathBuf, time::Duration};

use libafl::{StdFuzzer, prelude::*, schedulers::QueueScheduler, state::StdState};
use libafl_bolts::{current_nanos, rands::StdRand, tuples::tuple_list};

use crate::coverage::*;
use crate::feedback::multi_coverage_feedback::*;
use crate::harness;
use crate::monitor;
use crate::observer::multi_coverage_observer::MultiCoverageObserver;

fn jaccard_similarity(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());

    let (intersection, union) = a
        .iter()
        .zip(b.iter())
        .fold((0usize, 0usize), |(i, u), (a, b)| {
            (i + (a & b) as usize, u + (a | b) as usize)
        });

    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

fn distance_similarity(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());

    let dist_sq: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum();

    1.0 / (1.0 + dist_sq.sqrt())
}

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

pub(crate) struct CaseCoverage {
    pub coverage: HashMap<String, Vec<u8>>,
    pub is_passed: bool,
}

fn emit_top_passed_testcases(
    state: &StdState<
        InMemoryCorpus<ValueInput<Vec<u8>>>,
        ValueInput<Vec<u8>>,
        StdRand,
        OnDiskCorpus<ValueInput<Vec<u8>>>,
    >,
    init_cov: HashMap<String, Vec<u8>>,
    top_n: u64,
    corpus_output: Option<String>,
) -> Result<Vec<CaseCoverage>, Box<dyn std::error::Error>> {
    let corpus = state.corpus();
    let mut passed_cases = Vec::new();

    for id in corpus.ids() {
        let testcase = corpus.get(id)?.borrow();
        let metadata = testcase.metadata::<MultiCoverageMetadata>()?;

        if !metadata.is_passed {
            continue;
        }

        let similarity = init_cov
            .keys()
            .map(|cov_name| {
                jaccard_similarity(
                    &init_cov.get(cov_name).unwrap(),
                    &metadata.coverage.get(cov_name).unwrap(),
                )
            })
            .sum::<f64>()
            / init_cov.len() as f64;

        let input = testcase
            .input()
            .as_ref()
            .ok_or(Error::illegal_state(format!(
                "Corpus testcase {id} has no input"
            )))?;

        passed_cases.push((
            usize::from(id),
            input.clone(),
            metadata.coverage.clone(),
            similarity,
        ));
    }

    passed_cases.sort_by(|a, b| {
        b.3.partial_cmp(&a.3)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let limit = usize::min(top_n as usize, passed_cases.len());
    println!(
        "Found {} passed testcases with unique coverage, selecting top {}.",
        passed_cases.len(),
        limit
    );

    let top_passed_cases: Vec<_> = passed_cases.into_iter().take(limit).collect();
    for (rank, (id, input, _, similarity)) in top_passed_cases.iter().enumerate() {
        println!(
            "Top {} passed testcase: corpus_id={}, similarity={:.6}",
            rank + 1,
            id,
            similarity
        );

        if let Some(output_dir) = &corpus_output {
            let filename = format!("rank_{:04}_id_{}_sim_{:.6}", rank + 1, id, similarity);
            monitor::store_testcase(input, output_dir, Some(filename));
        }
    }

    let mut case_coverages: Vec<CaseCoverage> = top_passed_cases
        .into_iter()
        .map(|(_, _, cov, _)| CaseCoverage {
            coverage: cov,
            is_passed: true,
        })
        .collect();
    case_coverages.push(CaseCoverage {
        coverage: init_cov,
        is_passed: false,
    });

    Ok(case_coverages)
}

pub(crate) fn run_fuzzer(
    max_iters: u64,
    max_run_timeout: u64,
    top_n: u64,
    corpus_input: String,
    corpus_output: Option<String>,
) -> Result<Vec<CaseCoverage>, Box<dyn std::error::Error>> {
    // Scheduler, Feedback, Objective
    let scheduler = QueueScheduler::new();
    let observer = unsafe {
        MultiCoverageObserver::from_mut_ptr(
            "coverage",
            cover_names()
                .iter()
                .map(|cover_name| {
                    (
                        cover_name.to_owned(),
                        (cover_as_mut_ptr(cover_name), cover_len(cover_name)),
                    )
                })
                .collect(),
        )
    };
    let mut feedback = MultiCoverageFeedback::new(&observer);
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
        tuple_list!(observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
        Duration::from_secs(max_run_timeout),
    )
    .unwrap();

    // Fuzzing Loop
    let mutator = HavocScheduledMutator::new(havoc_mutations());
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    let init_bytes = load_initial_case(&corpus_input);
    let init_cov;
    if let (ExecuteInputResult::Corpus, Some(init_corpus_id)) =
        fuzzer.evaluate_input(&mut state, &mut executor, &mut mgr, &init_bytes)?
    {
        let mut init_testcase = state.corpus_mut().get(init_corpus_id)?.borrow_mut();
        let init_metadata = init_testcase.metadata_mut::<MultiCoverageMetadata>()?;
        init_metadata.is_initial = true;
        if init_metadata.is_passed {
            return Err(Box::new(Error::illegal_argument(format!(
                "Initial case from {corpus_input:?} did not crash"
            ))));
        }
        init_cov = init_metadata.coverage.clone();
    } else {
        return Err(Box::new(Error::illegal_argument(format!(
            "Initial case from {corpus_input:?} was not accepted into the main corpus by feedback"
        ))));
    }

    fuzzer.fuzz_loop_for(&mut stages, &mut executor, &mut state, &mut mgr, max_iters)?;

    emit_top_passed_testcases(&state, init_cov, top_n, corpus_output)
}
