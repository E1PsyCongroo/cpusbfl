// use libafl::prelude::*;

use crate::elf::*;
use crate::reduce::*;
use crate::state_tracker::*;

pub fn trim_after_max_pc(
    input: &[u8],
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    log::info!("Trimming after max PC...");

    let sections = parse_executable_sections(input)
        .inspect_err(|err| {
            log::warn!("Failed to parse executable ELF sections for trim reduction: {err}")
        })
        .ok()?;
    let max_pc = original
        .pc_tracker
        .iter()
        .map(|state| state.value)
        .max()
        .unwrap();
    let section = executable_section_containing_vma(
        &sections,
        max_pc,
        max_pc + u64::try_from(COMPRESSED_INST_BYTES).ok()?,
    )?;
    let max_offset = section.vma_to_offset(max_pc)?;
    let max_inst_len = inst_len_at(input, max_offset);
    let min_size = max_pc
        .checked_sub(section.vma_start)?
        .checked_add(max_inst_len as u64)?;
    let mut low = min_size;
    let mut high = section.size();
    let mut best = None;

    while low <= high {
        let mid = low + (high - low) / 2;
        let mut bytes = input.to_vec();
        write_executable_section_size(&mut bytes, section, mid)
            .inspect_err(|err| {
                log::warn!("Failed to write executable ELF section size for trim reduction: {err}")
            })
            .ok()?;

        match validate_exact_trace(BytesInput::from(bytes), original, original.len()) {
            Some((candidate, trackers)) => {
                best = Some((candidate, trackers));
                if mid == min_size {
                    break;
                }
                high = mid - 1;
            }
            None => {
                low = mid + 1;
            }
        }
    }

    best
}
