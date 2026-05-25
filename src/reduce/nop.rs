use std::collections::HashSet;

use libafl::prelude::*;

use crate::reduce::*;
use crate::state_tracker::*;

const C_NOP: [u8; 2] = [0x01, 0x00];

pub fn nop_unexecuted_insts(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    let executed = original.pc_tracker
        .iter()
        .map(|state| state.value)
        .collect::<HashSet<u64>>();
    let mut output = Vec::with_capacity(input.len());
    let mut offset = 0usize;

    while offset < input.len() {
        let pc = reset_vector + offset as u64;
        let inst_len = inst_len_at(input, offset);

        if executed.contains(&pc) {
            output.extend_from_slice(&input[offset..offset + inst_len]);
        } else {
            output.extend_from_slice(&C_NOP);
            if inst_len == 4 {
                output.extend_from_slice(&C_NOP);
            }
        }

        offset += inst_len;
    }

    validate_exact_trace(BytesInput::from(output), original, original.len())
}
