use std::collections::{BTreeMap, HashSet};

use libafl::prelude::*;

use crate::elf::*;
use crate::inst::*;
use crate::reduce::*;
use crate::state_tracker::*;

const SKIP_START_INST_NUM: usize = 20;
const CODE_SEGMENT_ALIGN: u64 = 0x1000;
const DEFAULT_SCRATCH_REG1: u8 = 1;
const DEFAULT_SCRATCH_REG2: u8 = 2;

#[derive(Debug)]
struct ContextRestorePlan {
    regs: Vec<(u8, u64)>,
    csrs: Vec<(u16, u64)>,
    memory_bytes: Vec<MemoryByte>,
    target_privilege: u64,
    privilege_changed: bool,
    mstatus: u64,
    scratches: [(u8, u64); 2],
}

fn mstatus_with_mpp(mstatus: u64, privilege_mode: u64) -> u64 {
    let mpp = match privilege_mode {
        0 | 1 | 3 => privilege_mode,
        _ => panic!("invalid privilege mode: {privilege_mode}"),
    };

    (mstatus & !(0b11 << 11)) | (mpp << 11)
}

fn write_bytes(memory: &mut BTreeMap<u64, u8>, write: MemoryWrite) {
    for offset in 0..write.access.bytes() {
        memory.insert(
            write.access.addr.wrapping_add(offset),
            (write.value >> (offset * 8)) as u8,
        );
    }
}

fn collect_memory_bytes(
    input: &[u8],
    elf_parser: &ELFParser,
    original: &StateTrackers,
    candidate_idx: usize,
) -> Option<Vec<MemoryByte>> {
    let pc_trace = original.pc_tracker.as_slice();
    let arch_trace = original.arch_int_reg_tracker.as_slice();
    let mut memory = BTreeMap::new();

    // Trace entry N is the state before instruction N. Replaying the skipped
    // prefix therefore includes instructions 0..N, excluding N itself.
    for idx in 0..candidate_idx {
        let pc = pc_trace.get(idx).unwrap().value;
        let regs = &arch_trace.get(idx).unwrap().value;
        let offset = usize::try_from(elf_parser.vma2offset(pc).unwrap()).unwrap();

        match decode_memory_write_at(input, offset, regs) {
            MemoryWriteDecode::NotStore => {}
            MemoryWriteDecode::Write(write) => write_bytes(&mut memory, write),
            MemoryWriteDecode::Unsupported => return None,
        }
    }

    if memory.is_empty() {
        return Some(Vec::new());
    }

    let mut needed = BTreeMap::new();
    for idx in candidate_idx..pc_trace.len() {
        let pc = pc_trace.get(idx).unwrap().value;
        let regs = &arch_trace.get(idx).unwrap().value;
        let offset = usize::try_from(elf_parser.vma2offset(pc).unwrap()).unwrap();

        match decode_memory_read_at(input, offset, regs) {
            MemoryReadDecode::NotLoad => {}
            MemoryReadDecode::Read(read) => {
                for byte_offset in 0..read.bytes() {
                    let addr = read.addr.wrapping_add(byte_offset);
                    if let Some(value) = memory.remove(&addr) {
                        needed.insert(addr, value);
                    }
                }
            }
            MemoryReadDecode::Unsupported => return None,
        }

        match decode_memory_write_at(input, offset, regs) {
            MemoryWriteDecode::NotStore => {}
            MemoryWriteDecode::Write(write) => {
                for byte_offset in 0..write.access.bytes() {
                    memory.remove(&write.access.addr.wrapping_add(byte_offset));
                }
            }
            MemoryWriteDecode::Unsupported => return None,
        }
    }

    Some(
        needed
            .into_iter()
            .map(|(addr, value)| MemoryByte { addr, value })
            .collect(),
    )
}

fn collect_context_restore_plan(
    original: &StateTrackers,
    input: &[u8],
    elf_parser: &ELFParser,
    candidate_idx: usize,
) -> Option<ContextRestorePlan> {
    let arch_trace = original.arch_int_reg_tracker.as_slice();
    let csr_trace = original.csr_tracker.as_slice();
    let arch = arch_trace.get(candidate_idx).unwrap();
    let csr = csr_trace.get(candidate_idx).unwrap();

    let mut reg_changed = [false; 32];
    for states in arch_trace.get(..=candidate_idx).unwrap().windows(2) {
        for (idx, (previous, current)) in states[0]
            .value
            .iter()
            .zip(states[1].value.iter())
            .enumerate()
            .skip(1)
        {
            reg_changed[idx] |= previous != current;
        }
    }

    let regs = reg_changed
        .iter()
        .enumerate()
        .filter_map(|(idx, &changed)| changed.then_some((idx as u8, arch.value[idx])))
        .collect::<Vec<_>>();
    let mut scratch_regs = regs.iter().map(|&(reg, _)| reg).take(2).collect::<Vec<_>>();
    for scratch_reg in [DEFAULT_SCRATCH_REG1, DEFAULT_SCRATCH_REG2] {
        if scratch_regs.len() == 2 {
            break;
        }
        if !scratch_regs.contains(&scratch_reg) {
            scratch_regs.push(scratch_reg);
        }
    }
    let scratches = [
        (scratch_regs[0], arch.value[usize::from(scratch_regs[0])]),
        (scratch_regs[1], arch.value[usize::from(scratch_regs[1])]),
    ];

    let csr_values = |state: &CSRState| {
        [
            (CSR_MSTATUS, state.mstatus),
            (CSR_MEPC, state.mepc),
            (CSR_SEPC, state.sepc),
            (CSR_MTVAL, state.mtval),
            (CSR_STVAL, state.stval),
            (CSR_MTVEC, state.mtvec),
            (CSR_STVEC, state.stvec),
            (CSR_MCAUSE, state.mcause),
            (CSR_SCAUSE, state.scause),
            (CSR_SATP, state.satp),
            (CSR_MIP, state.mip),
            (CSR_MIE, state.mie),
            (CSR_MSCRATCH, state.mscratch),
            (CSR_SSCRATCH, state.sscratch),
            (CSR_MIDELEG, state.mideleg),
            (CSR_MEDELEG, state.medeleg),
        ]
    };

    let privilege_changed = csr_trace.get(0)?.privilege_mode != csr.privilege_mode;
    let mut csr_changed = [false; 16];
    for states in csr_trace.get(..=candidate_idx)?.windows(2) {
        for (idx, ((_, previous), (_, current))) in csr_values(&states[0])
            .into_iter()
            .zip(csr_values(&states[1]))
            .enumerate()
        {
            csr_changed[idx] |= previous != current;
        }
    }

    let csrs = csr_values(csr)
        .into_iter()
        .enumerate()
        .filter_map(|(idx, value)| csr_changed[idx].then_some(value))
        .collect();

    Some(ContextRestorePlan {
        regs,
        csrs,
        memory_bytes: collect_memory_bytes(input, elf_parser, original, candidate_idx)?,
        target_privilege: csr.privilege_mode,
        privilege_changed,
        mstatus: csr.mstatus,
        scratches,
    })
}

fn nop_skipped_prefix_insts(
    bytes: &mut [u8],
    input: &[u8],
    elf_parser: &ELFParser,
    original: &StateTrackers,
    candidate_idx: usize,
) {
    let pc_trace = original.pc_tracker.as_slice();
    let suffix_pcs = pc_trace[candidate_idx..]
        .iter()
        .map(|state| state.value)
        .collect::<HashSet<_>>();
    let mut nopped = HashSet::new();

    for state in &pc_trace[1..candidate_idx] {
        if suffix_pcs.contains(&state.value) || !nopped.insert(state.value) {
            continue;
        }

        let offset = usize::try_from(elf_parser.vma2offset(state.value).unwrap()).unwrap();
        let inst_len = inst_len_at(input, offset);
        let end = offset.checked_add(inst_len).unwrap();

        match inst_len {
            2 => bytes[offset..end].copy_from_slice(&C_NOP),
            4 => bytes[offset..end].copy_from_slice(&NOP),
            _ => panic!("instruction length must be 2 or 4 bytes"),
        }
    }
}

fn append_context_restore(
    output: &mut Vec<u8>,
    context_pc: u64,
    plan: &ContextRestorePlan,
    target_pc: u64,
) -> Option<u64> {
    let use_mret = plan.privilege_changed && plan.target_privilege != 3;
    let addr_reg = plan.scratches[0].0;
    let value_reg = plan.scratches[1].0;
    let mut append_inst_count = 0;

    for &(csr_addr, csr_value) in &plan.csrs {
        if use_mret && csr_addr == CSR_MSTATUS {
            continue;
        }
        if use_mret && csr_addr == CSR_MEPC {
            continue;
        }
        append_inst_count += append_write_csr(output, csr_addr, csr_value, addr_reg);
    }

    if use_mret {
        append_inst_count += append_write_csr(
            output,
            CSR_MSTATUS,
            mstatus_with_mpp(plan.mstatus, plan.target_privilege),
            addr_reg,
        );

        // append li, csrrw, mret, min = 3
        let mut continuation_pc = context_pc + (append_inst_count + 3) * 4;
        let mut converged = false;
        for _ in 0..4 {
            let mepc_inst_count =
                append_write_csr(&mut Vec::new(), CSR_MEPC, continuation_pc, addr_reg);
            let next_continuation_pc = context_pc + (append_inst_count + mepc_inst_count + 1) * 4;
            if next_continuation_pc == continuation_pc {
                converged = true;
                break;
            }
            continuation_pc = next_continuation_pc;
        }
        if !converged {
            return None;
        }

        append_inst_count += append_write_csr(output, CSR_MEPC, continuation_pc, addr_reg);
        output.extend_from_slice(&MRET);
        append_inst_count += 1;
    }

    append_inst_count += append_memory_restore(output, &plan.memory_bytes, addr_reg, value_reg);

    for &(reg, value) in plan
        .regs
        .iter()
        .filter(|(reg, _)| *reg != addr_reg && *reg != value_reg)
    {
        append_inst_count += append_load_u64(output, reg, value);
    }

    for &(reg, value) in &plan.scratches {
        append_inst_count += append_load_u64(output, reg, value);
    }

    let jal_pc = context_pc + append_inst_count * 4;
    let jmp = encode_jmp(jal_pc, target_pc, false, None)?;
    output.extend_from_slice(&jmp);

    Some(append_inst_count + 1)
}

fn try_strip_prefix_at(
    input: &[u8],
    elf_parser: &ELFParser,
    candidate_idx: usize,
    candidate_pc: u64,
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    let entry_pc = original.pc_tracker.as_slice().first()?.value;
    let restore_plan = collect_context_restore_plan(original, input, elf_parser, candidate_idx)?;

    let mut context_pc = elf_parser
        .find_insert_vaddr(candidate_pc.abs_diff(entry_pc), CODE_SEGMENT_ALIGN)
        .ok()?;
    let mut context_code = Vec::new();
    let mut context_inst_count =
        append_context_restore(&mut context_code, context_pc, &restore_plan, candidate_pc)?;
    let mut converged = false;
    for _ in 0..4 {
        let next_context_pc = elf_parser
            .find_insert_vaddr(u64::try_from(context_code.len()).ok()?, CODE_SEGMENT_ALIGN)
            .ok()?;
        if next_context_pc == context_pc {
            converged = true;
            break;
        }
        context_code.clear();
        context_inst_count =
            append_context_restore(&mut context_code, context_pc, &restore_plan, candidate_pc)?;
        context_pc = next_context_pc;
    }
    if !converged {
        log::debug!(
            "Failed to converge context restore code placement, candidate_pc={:#x}",
            candidate_pc
        );
        return None;
    }

    let entry_jmp = encode_jmp(
        entry_pc,
        context_pc,
        true,
        Some(u64::from(restore_plan.scratches[0].0)),
    )?;
    let entry_offset = usize::try_from(elf_parser.vma2offset(entry_pc).unwrap()).unwrap();
    let entry_end = entry_offset.checked_add(entry_jmp.len()).unwrap();

    let mut bytes = input.to_vec();
    bytes
        .get_mut(entry_offset..entry_end)?
        .copy_from_slice(&entry_jmp);
    nop_skipped_prefix_insts(&mut bytes, input, elf_parser, original, candidate_idx);

    let bytes = ELFParser::from_bytes(&bytes)
        .ok()?
        .insert_code_segment(&context_code, context_pc, CODE_SEGMENT_ALIGN)
        .inspect_err(|err| log::warn!("Failed to insert prefix restore code: {err}"))
        .ok()?
        .into_bytes()
        .inspect_err(|err| log::warn!("Failed to serialize prefix-reduced ELF: {err}"))
        .ok()?;

    let suffix_inst_count = original.len().saturating_sub(candidate_idx);
    let max_inst = entry_jmp.len() / STANDARD_INST_BYTES
        + usize::try_from(context_inst_count).ok()?
        + suffix_inst_count;

    log::debug!(
        "Prefix-reduced candidate: entry_pc={:#x}, context_pc={:#x}, candidate_pc={:#x}, \
         prefix_inst_count={}, context_inst_count={}, suffix_inst_count={}, max_inst={}",
        entry_pc,
        context_pc,
        candidate_pc,
        candidate_idx,
        context_inst_count,
        suffix_inst_count,
        max_inst
    );

    let bytes = BytesInput::from(bytes);
    // let filename = format!("prefix_{candidate_idx}");
    // store_testcase(&bytes, None, "reduce", Some(&filename)).unwrap();

    validate_exact_trace(bytes, original, max_inst)
}

pub fn strip_irrelevant_prefix(
    input: &[u8],
    original: &StateTrackers,
) -> Option<(BytesInput, StateTrackers)> {
    log::info!("Stripping irrelevant prefix, input_size={:#x}", input.len());

    let pc_trace = &original.pc_tracker;
    let elf_parser = ELFParser::from_bytes(input)
        .inspect_err(|err| log::warn!("Failed to parse ELF for strip prefix reduction: {err}"))
        .ok()?;
    let candidates: Vec<(usize, u64)> = pc_trace
        .iter()
        .enumerate()
        .filter_map(|(idx, pc)| (idx >= SKIP_START_INST_NUM).then(|| (idx, pc.value)))
        .collect();

    if let Some((candidate, trackers)) = try_strip_prefix_at(
        input,
        &elf_parser,
        candidates.last()?.0,
        candidates.last()?.1,
        original,
    ) {
        return Some((candidate, trackers));
    };

    let mut low = 0usize;
    let mut high = candidates.len() - 1;
    let mut best = None;

    while low < high {
        let mid = low + (high - low) / 2;
        let (candidate_idx, candidate_pc) = candidates[mid];
        match try_strip_prefix_at(input, &elf_parser, candidate_idx, candidate_pc, original) {
            Some((candidate, trackers)) => {
                best = Some((candidate, trackers));
                low = mid + 1;
                log::debug!(
                    "Found prefix-reduced candidate at index {candidate_idx}, pc={:#x}",
                    candidate_pc
                );
            }
            None => {
                high = mid;
                log::debug!(
                    "Failed to reduce prefix at index {candidate_idx}, pc={:#x}",
                    candidate_pc
                );
            }
        }
    }

    best
}
