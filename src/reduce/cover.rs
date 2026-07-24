use std::collections::HashSet;

use libafl::prelude::*;

use super::run_and_collect;

use crate::elf::ELFParser;
use crate::fuzzer::CaseMetadata;
use crate::inst::*;
use crate::state_tracker::*;

fn nopped_by_pcs(input: &BytesInput, pcs: &HashSet<u64>) -> Option<BytesInput> {
    let bytes = input.mutator_bytes();
    let elf_parser = ELFParser::from_bytes(bytes)
        .inspect_err(|err| log::warn!("Failed to parse ELF for cover reduction: {err}"))
        .ok()?;
    let mut nopped = bytes.to_vec();

    for pc in pcs {
        let offset = usize::try_from(elf_parser.vma2offset(*pc).ok()?).ok()?;
        let inst_len = inst_len_at(bytes, offset);

        match inst_len {
            2 => nopped[offset..offset + inst_len].copy_from_slice(&C_NOP),
            4 => nopped[offset..offset + inst_len].copy_from_slice(&NOP),
            _ => return None,
        }
    }

    Some(BytesInput::from(nopped))
}

fn nopped_by_pc(input: &BytesInput, pc: u64) -> Option<BytesInput> {
    nopped_by_pcs(input, &HashSet::from([pc]))
}

fn pc_appears_more_than_once(pc_tracker: &StateTracker<PCState>, pc: u64) -> bool {
    pc_tracker.iter().filter(|state| state.value == pc).count() > 1
}

fn nopped_suffix_pcs(
    input: &BytesInput,
    pc_tracker: &StateTracker<PCState>,
    prefix_len: usize,
) -> Option<BytesInput> {
    let suffix_pcs = pc_tracker
        .iter()
        .skip(prefix_len)
        .map(|state| state.value)
        .collect::<HashSet<_>>();

    nopped_by_pcs(input, &suffix_pcs)
}

fn common_prefix_len(
    init_pc_tracker: &StateTracker<PCState>,
    pass_pc_tracker: &StateTracker<PCState>,
    mutated_pcs: &HashSet<u64>,
) -> usize {
    init_pc_tracker
        .iter()
        .zip(pass_pc_tracker.iter())
        .take_while(|(init_pc, pass_pc)| {
            init_pc == pass_pc && !mutated_pcs.contains(&pass_pc.value)
        })
        .count()
}

fn max_prefix_len_preserving_suffix_nop(
    pass_pc_tracker: &StateTracker<PCState>,
    mut prefix_len: usize,
) -> usize {
    loop {
        let suffix_pcs = pass_pc_tracker
            .iter()
            .skip(prefix_len)
            .map(|state| state.value)
            .collect::<HashSet<_>>();

        let Some(conflict_idx) = pass_pc_tracker
            .iter()
            .take(prefix_len)
            .position(|state| suffix_pcs.contains(&state.value))
        else {
            return prefix_len;
        };

        log::debug!(
            "Pass cover reduction shortening prefix from {prefix_len} to {conflict_idx}: \
             PC {:#x} appears again in suffix",
            pass_pc_tracker.as_slice()[conflict_idx].value
        );
        prefix_len = conflict_idx;
    }
}

pub(crate) fn reduce_init_case_coverage(input: &BytesInput, metadata: &mut CaseMetadata) {
    let original_len = metadata.state_trackers.len();
    if original_len <= 1 {
        log::info!("Init cover reduction skipped: trace is too short");
        return;
    }

    let Some(last_pc) = metadata
        .state_trackers
        .pc_tracker
        .as_slice()
        .last()
        .map(|state| state.value)
    else {
        log::info!("Init cover reduction skipped: empty PC tracker");
        return;
    };

    if pc_appears_more_than_once(&metadata.state_trackers.pc_tracker, last_pc) {
        log::info!(
            "Init cover reduction skipped: last PC {last_pc:#x} appears more than once in the dynamic trace"
        );
        return;
    }

    let Some(nopped_input) = nopped_by_pc(input, last_pc) else {
        log::info!("Init cover reduction failed: could not NOP last instruction");
        return;
    };

    let max_inst = original_len - 1;
    let (exit_kind, nopped_covers, _) = run_and_collect(&nopped_input, max_inst);

    match exit_kind {
        ExitKind::Crash => {
            log::info!(
                "Init cover reduction succeeded: replacing coverage with last-NOP crash run"
            );
            metadata.covers = nopped_covers;
        }
        ExitKind::Ok => {
            log::info!("Init cover reduction succeeded: subtracting last-NOP passing run");
            metadata.covers = metadata.covers.saturating_sub(&nopped_covers);
        }
        _ => {
            panic!("Init cover reduction failed: unexpected exit kind from last-NOP run");
        }
    }
}

pub(crate) fn reduce_pass_case_coverage(
    input: &BytesInput,
    init_pc_tracker: &StateTracker<PCState>,
    metadata: &mut CaseMetadata,
) {
    let Some(mutated_pcs) = metadata.mutated_pcs.as_ref().filter(|pcs| !pcs.is_empty()) else {
        log::info!("Pass cover reduction skipped: missing mutation PC history");
        return;
    };

    let mut prefix_len = max_prefix_len_preserving_suffix_nop(
        &metadata.state_trackers.pc_tracker,
        common_prefix_len(
            init_pc_tracker,
            &metadata.state_trackers.pc_tracker,
            mutated_pcs,
        ),
    );

    while prefix_len > 0 {
        let Some(nopped_input) =
            nopped_suffix_pcs(input, &metadata.state_trackers.pc_tracker, prefix_len)
        else {
            log::info!("Pass cover reduction failed: could not build NOP input");
            return;
        };

        let (exit_kind, common_covers, _) = run_and_collect(&nopped_input, prefix_len);

        match exit_kind {
            ExitKind::Crash => {
                log::info!("Pass cover reduction failed at prefix {prefix_len}: crash run")
            }
            ExitKind::Ok => {
                log::info!(
                    "Pass cover reduction succeeded: subtracted common prefix of {prefix_len} instructions"
                );
                metadata.covers = metadata.covers.saturating_sub(&common_covers);
                return;
            }
            _ => {
                panic!("Pass cover reduction failed: unexpected exit kind from last-NOP run");
            }
        }

        prefix_len = max_prefix_len_preserving_suffix_nop(
            &metadata.state_trackers.pc_tracker,
            prefix_len - 1,
        );
    }

    log::info!("Pass cover reduction failed: no stable common dynamic prefix");
}
