use libafl::prelude::*;

use crate::reduce::*;
use crate::state_tracker::*;

pub fn trim_after_max_pc(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    let max_pc = original
        .pc_tracker
        .iter()
        .map(|state| state.value)
        .max()
        .unwrap();
    let max_offset = pc_to_offset(input, reset_vector, max_pc);
    let max_inst_len = inst_len_at(input, max_offset);

    validate_exact_trace(
        BytesInput::from(input[..max_offset + max_inst_len].to_vec()),
        original,
        original.len(),
    )
}
