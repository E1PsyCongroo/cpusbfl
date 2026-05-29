use std::collections::HashSet;

use libafl::prelude::*;

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
    let max_inst = insts_to_failure_after_jal(&original.pc_tracker, candidate_pc);

    validate_exact_trace(BytesInput::from(bytes), original, max_inst)
}

pub fn strip_irrelevant_suffix(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    println!("Stripping irrelevant suffix...");
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
