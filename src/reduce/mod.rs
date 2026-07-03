mod nop;
mod strip_prefix;
mod strip_suffix;
mod trim;

use libafl::prelude::*;

use crate::elf::*;
use crate::harness::{sim_run_with_trackers, sim_with_max_inst};
use crate::inst::*;
use crate::state_tracker::*;
use crate::utils::*;
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

fn validate_exact_trace(
    input: BytesInput,
    original: &StateTrackers,
    max_inst: usize,
) -> Option<(BytesInput, StateTrackers)> {
    let (exit_kind, candidate) = run_and_collect(&input, max_inst);
    is_same_failure_site(exit_kind, &candidate, original).then(|| (input, candidate))
}

pub(crate) fn reduce_fault_case(
    input: BytesInput,
    original: StateTrackers,
    save_reduce: bool,
    output_dir: &Option<String>,
) -> BytesInput {
    assert!(original.len() > 0);
    let output_dir = output_dir.as_deref();

    let stripped_input = ELFParser::from_bytes(input.mutator_bytes())
        .inspect_err(|e| log::warn!("Failed to parse ELF input for reduction: {e}"))
        .and_then(|elf_parser| {
            elf_parser
                .strip()
                .into_bytes()
                .inspect_err(|e| log::warn!("Failed to strip ELF input for reduction: {e}"))
        })
        .map(BytesInput::from)
        .unwrap_or(input);

    let (nopped_bytes, nopped_trackers) =
        match nop_unexecuted_insts(stripped_input.mutator_bytes(), &original) {
            Some((bytes, trackers)) => {
                log::info!("Nop unexecuted insts successed");
                if save_reduce && output_dir.is_some() {
                    store_testcase(&bytes, None, output_dir.unwrap(), Some("init_nopped")).unwrap();
                }
                (bytes, trackers)
            }
            None => {
                log::info!("Nop unexecuted insts failed, skipping");
                (stripped_input, original.to_owned())
            }
        };

    let (stripped_prefix_bytes, stripped_prefix_trackers) =
        match strip_irrelevant_prefix(nopped_bytes.mutator_bytes(), &nopped_trackers) {
            Some((bytes, trackers)) => {
                log::info!("Strip irrelevant prefix insts successed");
                if save_reduce && output_dir.is_some() {
                    store_testcase(
                        &bytes,
                        None,
                        output_dir.unwrap(),
                        Some("init_striped_prefix"),
                    )
                    .unwrap();
                }
                (bytes, trackers)
            }
            None => {
                log::info!("Strip irrelevant prefix insts failed, skipping");
                (nopped_bytes, nopped_trackers)
            }
        };

    let (stripped_suffix_bytes, stripped_suffix_trackers) = match strip_irrelevant_suffix(
        stripped_prefix_bytes.mutator_bytes(),
        &stripped_prefix_trackers,
    ) {
        Some((bytes, trackers)) => {
            log::info!("Strip irrelevant suffix insts successed");
            if save_reduce && output_dir.is_some() {
                store_testcase(
                    &bytes,
                    None,
                    output_dir.unwrap(),
                    Some("init_striped_suffix"),
                )
                .unwrap();
            }
            (bytes, trackers)
        }
        None => {
            log::info!("Strip irrelevant suffix insts failed, skipping");
            (stripped_prefix_bytes, stripped_prefix_trackers)
        }
    };

    // let (trimmed_bytes, trimmed_trackers) = match trim_after_max_pc(
    //     stripped_suffix_bytes.mutator_bytes(),
    //     &stripped_suffix_trackers,
    // ) {
    //     Some((bytes, trackers)) => {
    //         log::info!("Trim case after max pc successed");
    //         if save_reduce && output_dir.is_some() {
    //             store_testcase(&bytes, None, output_dir.unwrap(), Some("init_trimmed")).unwrap();
    //         }
    //         (bytes, trackers)
    //     }
    //     None => (stripped_suffix_bytes, stripped_suffix_trackers),
    // };

    stripped_suffix_bytes
}
