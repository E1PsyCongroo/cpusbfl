use libafl::prelude::*;
use lief;
use lief::generic::Section;

use crate::elf::*;
use crate::inst::{C_NOP, NOP};
use crate::reduce::*;
use crate::state_tracker::*;

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
    elf_parser: &ELFParser,
    original: &StateTrackers,
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

        let section = elf_parser.section_containing_vma(
            state.value,
            state.value.checked_add(COMPRESSED_INST_BYTES as u64)?,
        )?;
        let offset = usize::try_from(
            state
                .value
                .checked_sub(section.virtual_address())?
                .checked_add(section.offset())?,
        )
        .ok()?;
        let section_file_end =
            usize::try_from(section.offset().checked_add(section.size())?).ok()?;

        if offset.checked_add(COMPRESSED_INST_BYTES)? > section_file_end || !nopped.insert(offset) {
            continue;
        }

        let inst_len = inst_len_at(input, offset);
        let end = offset.checked_add(inst_len)?;
        if end > section_file_end {
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
    elf_parser: &ELFParser,
    candidate_pc: u64,
    failure_pc: u64,
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    let mut bytes = input.to_vec();
    let offset = usize::try_from(elf_parser.vma2offset(
        candidate_pc,
        candidate_pc.checked_add(STANDARD_INST_BYTES as u64)?,
    )?)
    .ok()?;

    let jmp = encode_jmp(candidate_pc, failure_pc, false, None)?;
    bytes[offset..offset.checked_add(STANDARD_INST_BYTES)?].copy_from_slice(&jmp);
    nop_skipped_suffix_insts(
        &mut bytes,
        input,
        elf_parser,
        original,
        candidate_pc,
        failure_pc,
    )?;
    let max_inst = insts_to_failure_after_jal(&original.pc_tracker, candidate_pc);

    validate_exact_trace(BytesInput::from(bytes), original, max_inst)
}

pub fn strip_irrelevant_suffix(
    input: &[u8],
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    log::info!("Stripping irrelevant suffix, input_size={:#x}", input.len());

    let pc_trace = &original.pc_tracker;
    let failure_pc = pc_trace.as_slice().last().unwrap().value;
    let candidates: Vec<u64> = first_dynamic_entries(pc_trace)
        .into_iter()
        .map(|(_, pc)| pc)
        .filter(|&pc| {
            pc != failure_pc
                && pc.checked_add(COMPRESSED_INST_BYTES as u64) != Some(failure_pc)
                && encode_jmp(pc, failure_pc, false, None).is_some()
        })
        .collect();

    let elf_parse = ELFParser::from_bytes(input)
        .inspect_err(|err| log::warn!("Failed to parse ELF for strip suffix reduction: {err}"))
        .ok()?;

    let mut low = 0usize;
    let mut high = candidates.len();
    let mut best = None;

    while low < high {
        let mid = low + (high - low) / 2;
        match try_jal_to_failure_site(input, &elf_parse, candidates[mid], failure_pc, original) {
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
