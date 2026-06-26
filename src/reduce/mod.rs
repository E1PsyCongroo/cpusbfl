mod inst;
mod nop;
mod strip_prefix;
mod strip_suffix;
mod trim;

use libafl::prelude::*;

use crate::harness::{sim_run_with_trackers, sim_with_max_inst};
use crate::monitor::*;
use crate::state_tracker::*;
use inst::*;
use nop::nop_unexecuted_insts;
use strip_prefix::strip_irrelevant_prefix;
use strip_suffix::strip_irrelevant_suffix;
use trim::trim_after_max_pc;

fn run_and_collect(input: &BytesInput, max_inst: usize) -> (ExitKind, StateTrackers) {
    sim_with_max_inst(max_inst, || {
        let exit_kind = sim_run_with_trackers(input);
        let state_trackers = trackers().clone();
        (exit_kind, state_trackers)
    })
}

fn is_same_failure_site(
    exit_kind: ExitKind,
    candidate: &StateTrackers,
    original: &StateTrackers,
) -> bool {
    if !matches!(exit_kind, ExitKind::Crash) {
        return false;
    }

    let Some(candidate_pc) = candidate.pc_tracker.as_slice().last() else {
        return false;
    };
    let Some(original_pc) = original.pc_tracker.as_slice().last() else {
        return false;
    };

    candidate_pc == original_pc
}

pub fn validate_exact_trace(
    input: BytesInput,
    original: &StateTrackers,
    max_inst: usize,
) -> Option<(BytesInput, StateTrackers)> {
    let (exit_kind, candidate) = run_and_collect(&input, max_inst);
    if is_same_failure_site(exit_kind, &candidate, original) {
        Some((input, candidate))
    } else {
        None
    }
}

pub(crate) fn reduce_fault_case(
    input: &BytesInput,
    original: &StateTrackers,
    save_reduce: bool,
    output_dir: &Option<String>,
) -> BytesInput {
    assert!(original.len() > 0);
    let output_dir = output_dir.as_deref();

    let trimmed_bytes = match trim_after_max_pc(input.mutator_bytes(), &original) {
        Some((bytes, _)) => {
            log::info!("Trim case after max pc successed");
            if save_reduce && output_dir.is_some() {
                store_testcase(&bytes, None, output_dir.unwrap(), Some("init_trimmed"));
            }
            bytes
        }
        None => input.to_owned(),
    };

    let nopped_bytes =
        match nop_unexecuted_insts(trimmed_bytes.mutator_bytes(), &original) {
            Some((bytes, _)) => {
                log::info!("Nop unexecuted insts successed");
                if save_reduce && output_dir.is_some() {
                    store_testcase(&bytes, None, output_dir.unwrap(), Some("init_nopped"));
                }
                bytes
            }
            None => trimmed_bytes.to_owned(),
        };

    let (striped_suffix_bytes, _striped_suffix_trackers) =
        match strip_irrelevant_suffix(nopped_bytes.mutator_bytes(), original) {
            Some((bytes, trackers)) => {
                log::info!("Strip irrelevant suffix insts successed");
                if save_reduce && output_dir.is_some() {
                    store_testcase(
                        &bytes,
                        None,
                        output_dir.unwrap(),
                        Some("init_striped_suffix"),
                    );
                }
                (bytes, trackers)
            }
            None => (nopped_bytes.to_owned(), original.to_owned()),
        };

    // let striped_prefix_bytes = match strip_irrelevant_prefix(
    //     striped_suffix_bytes.mutator_bytes(),
    //     &striped_suffix_trackers,
    //     reset_vector,
    // ) {
    //     Some((bytes, _)) => {
    //         log::info!("Strip irrelevant prefix insts successed");
    //         if save_reduce && output_dir.is_some() {
    //             store_testcase(
    //                 &bytes,
    //                 None,
    //                 output_dir.unwrap(),
    //                 Some("init_striped_prefix"),
    //             );
    //         }
    //         bytes
    //     }
    //     None => striped_suffix_bytes.to_owned(),
    // };

    // striped_prefix_bytes
    striped_suffix_bytes
}
