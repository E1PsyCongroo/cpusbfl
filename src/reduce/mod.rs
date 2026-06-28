mod inst;
mod nop;
mod strip_prefix;
mod strip_suffix;
mod trim;

use std::collections::HashSet;

use libafl::prelude::*;

use crate::elf::*;
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

pub fn first_dynamic_entries(pc_trace: &StateTracker<PCState>) -> Vec<(usize, u64)> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for (idx, state) in pc_trace.iter().enumerate() {
        if seen.insert(state.value) {
            entries.push((idx, state.value));
        }
    }

    entries
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
                    store_testcase(&bytes, None, output_dir.unwrap(), Some("init_nopped"));
                }
                (bytes, trackers)
            }
            None => (stripped_input, original.to_owned()),
        };

    let (striped_suffix_bytes, striped_suffix_trackers) =
        match strip_irrelevant_suffix(nopped_bytes.mutator_bytes(), &nopped_trackers) {
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
            None => (nopped_bytes.to_owned(), nopped_trackers.to_owned()),
        };

    let (trimmed_bytes, trimmed_trackers) = match trim_after_max_pc(
        striped_suffix_bytes.mutator_bytes(),
        &striped_suffix_trackers,
    ) {
        Some((bytes, trackers)) => {
            log::info!("Trim case after max pc successed");
            if save_reduce && output_dir.is_some() {
                store_testcase(&bytes, None, output_dir.unwrap(), Some("init_trimmed"));
            }
            (bytes, trackers)
        }
        None => (
            striped_suffix_bytes.to_owned(),
            striped_suffix_trackers.to_owned(),
        ),
    };

    match strip_irrelevant_prefix(trimmed_bytes.mutator_bytes(), &trimmed_trackers) {
        Some((bytes, _)) => {
            log::info!("Strip irrelevant prefix insts successed");
            if save_reduce && output_dir.is_some() {
                store_testcase(
                    &bytes,
                    None,
                    output_dir.unwrap(),
                    Some("init_striped_prefix"),
                );
            }
            bytes
        }
        None => trimmed_bytes,
    }
}
