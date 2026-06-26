use std::collections::HashSet;

use libafl::prelude::*;

use crate::elf::*;
use crate::reduce::*;
use crate::state_tracker::*;

#[derive(Clone, Copy)]
struct CodeInst {
    pc: u64,
    offset: usize,
    len: usize,
}

fn collect_code_insts(input: &[u8], sections: &[ExecutableSection]) -> Option<Vec<CodeInst>> {
    let mut insts = Vec::new();

    for section in sections {
        let mut offset = section.file_offset;
        let mut pc = section.vma_start;

        while offset + 2 <= section.file_end && pc < section.vma_end {
            let inst_len = inst_len_at(input, offset);
            if offset + inst_len > section.file_end {
                break;
            }
            insts.push(CodeInst {
                pc,
                offset,
                len: inst_len,
            });
            offset += inst_len;
            pc += inst_len as u64;
        }
    }

    Some(insts)
}

fn build_nopped_input(
    input: &[u8],
    insts: &[CodeInst],
    executed: &HashSet<u64>,
    keep_window: Option<usize>,
) -> BytesInput {
    let mut keep = vec![false; insts.len()];

    for (idx, inst) in insts.iter().enumerate() {
        if executed.contains(&inst.pc) {
            keep[idx] = true;
            if let Some(window) = keep_window {
                let start = idx.saturating_sub(window);
                let end = usize::min(idx + window + 1, insts.len());
                keep[start..end].fill(true);
            }
        }
    }

    let mut output = input.to_vec();
    for (idx, inst) in insts.iter().enumerate() {
        if keep[idx] {
            continue;
        }

        match inst.len {
            2 => output[inst.offset..inst.offset + inst.len].copy_from_slice(&C_NOP),
            4 => output[inst.offset..inst.offset + inst.len].copy_from_slice(&NOP),
            _ => panic!("instruction length must be 2 or 4 bytes"),
        }
    }

    BytesInput::from(output)
}

fn try_keep_window(
    input: &[u8],
    insts: &[CodeInst],
    executed: &HashSet<u64>,
    original: &StateTrackers,
    keep_window: Option<usize>,
) -> Option<(BytesInput, StateTrackers)> {
    validate_exact_trace(
        build_nopped_input(input, insts, executed, keep_window),
        original,
        original.len(),
    )
}

pub fn nop_unexecuted_insts(
    input: &[u8],
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    log::info!("Nopping unexecuted instructions...");

    let sections = parse_executable_sections(input)
        .inspect_err(|err| {
            log::warn!("Failed to parse executable ELF sections for nop reduction: {err}")
        })
        .ok()?;
    let insts = collect_code_insts(input, &sections)?;

    let executed = original
        .pc_tracker
        .iter()
        .map(|state| state.value)
        .collect::<HashSet<u64>>();

    if let Some(result) = try_keep_window(input, &insts, &executed, original, None) {
        return Some(result);
    }

    if insts.is_empty() {
        return None;
    }

    let mut last_failed = 0usize;
    let mut high = 1usize;
    let mut best = loop {
        match try_keep_window(input, &insts, &executed, original, Some(high)) {
            Some(result) => {
                break result;
            }
            None if high >= insts.len() => return None,
            None => {
                last_failed = high;
                high = usize::min(high.saturating_mul(2), insts.len());
            }
        }
    };

    let mut low = last_failed + 1;
    while low < high {
        let mid = low + (high - low) / 2;
        match try_keep_window(input, &insts, &executed, original, Some(mid)) {
            Some(result) => {
                best = result;
                high = mid;
            }
            None => {
                low = mid + 1;
            }
        }
    }

    Some(best)
}
