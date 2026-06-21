use std::collections::HashSet;

use libafl::prelude::*;

use super::inst::{C_NOP, NOP};
use crate::reduce::*;
use crate::state_tracker::*;

fn first_dynamic_pcs(pc_trace: &StateTracker<PCState>) -> Vec<u64> {
    let mut seen = HashSet::new();
    let mut pcs = Vec::new();

    for state in pc_trace.iter() {
        if seen.insert(state.value) {
            pcs.push(state.value);
        }
    }

    pcs
}

fn insts_to_failure_after_jal(pc_trace: &StateTracker<PCState>, candidate_pc: u64) -> usize {
    let candidate_idx = pc_trace
        .iter()
        .position(|state| state.value == candidate_pc)
        .expect("candidate pc must come from the dynamic trace");

    candidate_idx + 2
}

fn nop_skipped_suffix_insts(
    bytes: &mut [u8],
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
    candidate_pc: u64,
    failure_pc: u64,
) -> Option<()> {
    let pc_trace = original.pc_tracker.as_slice();
    let candidate_idx = pc_trace
        .iter()
        .position(|state| state.value == candidate_pc)?;
    let prefix_pcs = pc_trace[..=candidate_idx]
        .iter()
        .map(|state| state.value)
        .collect::<HashSet<_>>();
    let mut nopped = HashSet::new();

    for state in &pc_trace[candidate_idx + 1..pc_trace.len().saturating_sub(1)] {
        if state.value == failure_pc || prefix_pcs.contains(&state.value) {
            continue;
        }

        let offset = usize::try_from(state.value.checked_sub(reset_vector)?).ok()?;
        if offset.checked_add(2)? > input.len() || !nopped.insert(offset) {
            continue;
        }

        let inst_len = inst_len_at(input, offset);
        let end = offset.checked_add(inst_len)?;
        if end > bytes.len() {
            return None;
        }

        match inst_len {
            2 => bytes[offset..end].copy_from_slice(&C_NOP),
            4 => bytes[offset..end].copy_from_slice(&NOP),
            _ => panic!("instruction length must be 2 or 4 bytes"),
        }
    }

    Some(())
}

fn try_jal_to_failure_site(
    input: &[u8],
    candidate_pc: u64,
    failure_pc: u64,
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    let mut bytes = input.to_vec();
    let offset = pc_to_offset(&bytes, reset_vector, candidate_pc);

    let jmp = encode_jmp(candidate_pc, failure_pc, false, None)?;
    assert!(jmp.len() == 4, "only support 4-byte jmp for now");
    bytes[offset..offset + 4].copy_from_slice(&jmp);
    nop_skipped_suffix_insts(
        &mut bytes,
        input,
        original,
        reset_vector,
        candidate_pc,
        failure_pc,
    )?;
    let max_inst = insts_to_failure_after_jal(&original.pc_tracker, candidate_pc);

    validate_exact_trace(BytesInput::from(bytes), original, max_inst)
}

pub fn strip_irrelevant_suffix(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    log::info!("Stripping irrelevant suffix...");
    let pc_trace = &original.pc_tracker;
    assert!(pc_trace.len() > 0);

    let failure_pc = pc_trace.as_slice().last().unwrap().value;
    let candidates: Vec<u64> = first_dynamic_pcs(pc_trace)
        .into_iter()
        .filter(|&pc| {
            pc != failure_pc
                && pc + 2 != failure_pc
                && encode_jmp(pc, failure_pc, false, None).is_some()
        })
        .collect();

    let mut low = 0usize;
    let mut high = candidates.len();
    let mut best = None;

    while low < high {
        let mid = low + (high - low) / 2;
        match try_jal_to_failure_site(input, candidates[mid], failure_pc, original, reset_vector) {
            Some((candidate, trackers)) => {
                best = Some((candidate, trackers));
                high = mid;
            }
            None => {
                low = mid + 1;
            }
        }
    }

    best
}
