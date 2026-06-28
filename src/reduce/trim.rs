use lief::generic::Section;

use crate::elf::*;
use crate::reduce::*;
use crate::state_tracker::*;

pub fn trim_after_max_pc(
    input: &[u8],
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    log::info!("Start trimming after max PC, input_size={:#x}", input.len());

    let mut elf_parser = ELFParser::from_bytes(input)
        .inspect_err(|err| log::warn!("Failed to parse ELF for trim reduction: {err}"))
        .ok()?;
    let max_pc = original.pc_tracker.iter().map(|state| state.value).max()?;

    let section = elf_parser.section_containing_vma(
        max_pc,
        max_pc.checked_add(u64::try_from(COMPRESSED_INST_BYTES).ok()?)?,
    )?;
    let section_name = section.name();
    let max_offset = max_pc
        .checked_sub(section.virtual_address())?
        .checked_add(section.offset())?;
    let max_inst_len = u64::try_from(inst_len_at(input, usize::try_from(max_offset).ok()?)).ok()?;

    assert!(section_contains_range(
        section,
        max_pc,
        max_pc.checked_add(max_inst_len)?
    ));

    let mut low = max_pc
        .checked_sub(section.virtual_address())?
        .checked_add(max_inst_len)?;
    let mut high = section.size();
    let mut best = None;
    while low <= high {
        let mid = low + (high - low) / 2;
        let bytes = elf_parser
            .reduce_section_by_name(&section_name, mid)
            .inspect_err(|err| log::warn!("Failed to trim reduce section '{section_name}': {err}"))
            .ok()?
            .into_bytes()
            .inspect_err(|err| log::warn!("Failed to serialize trim-reduced ELF: {err}",))
            .ok()?;

        match validate_exact_trace(BytesInput::from(bytes), original, original.len()) {
            Some((candidate, trackers)) => {
                best = Some((candidate, trackers));
                high = mid - 1;
            }
            None => {
                low = mid + 1;
            }
        }

        elf_parser = ELFParser::from_bytes(input).unwrap();
    }

    best
}
