use std::collections::HashSet;

use libafl::prelude::*;

use super::inst::*;
use crate::reduce::*;
use crate::state_tracker::*;

struct ContextRestorePlan {
    regs: Vec<(u8, u64)>,
    csrs: Vec<(u16, u64)>,
    memory_writes: Vec<MemoryWrite>,
    target_privilege: u64,
}

fn first_dynamic_entries(pc_trace: &StateTracker<PCState>) -> Vec<(usize, u64)> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for (idx, state) in pc_trace.iter().enumerate() {
        if seen.insert(state.value) {
            entries.push((idx, state.value));
        }
    }

    entries
}

fn mstatus_with_mpp(mstatus: u64, privilege_mode: u64) -> u64 {
    let mpp = match privilege_mode {
        0 | 1 | 3 => privilege_mode,
        _ => panic!("invalid privilege mode: {privilege_mode}"),
    };

    (mstatus & !(0b11 << 11)) | (mpp << 11)
}

fn collect_memory_writes(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
    restore_idx: usize,
) -> Option<Vec<MemoryWrite>> {
    let pc_trace = original.pc_tracker.as_slice();
    let arch_trace = original.arch_int_reg_tracker.as_slice();
    let mut writes = Vec::new();

    for idx in 0..=restore_idx {
        let pc = pc_trace.get(idx)?.value;
        let regs = &arch_trace.get(idx)?.value;
        let offset = usize::try_from(pc.checked_sub(reset_vector)?).ok()?;

        match decode_memory_write_at(input, offset, regs) {
            MemoryWriteDecode::NotStore => {}
            MemoryWriteDecode::Write(write) => writes.push(write),
            MemoryWriteDecode::Unsupported => return None,
        }
    }

    Some(writes)
}

fn collect_context_restore_plan(
    original: &StateTrackers,
    input: &[u8],
    restore_idx: usize,
    reset_vector: u64,
) -> Option<ContextRestorePlan> {
    let arch = original.arch_int_reg_tracker.as_slice().get(restore_idx)?;
    let csr = original.csr_tracker.as_slice().get(restore_idx)?;

    let regs = arch
        .value
        .iter()
        .enumerate()
        .filter_map(|(idx, &value)| (idx != 0).then(|| (idx as u8, value)))
        .collect::<Vec<_>>();

    let csrs: Vec<(u16, u64)> = vec![
        (CSR_MSTATUS, csr.mstatus),
        (CSR_MEPC, csr.mepc),
        (CSR_SEPC, csr.sepc),
        (CSR_MTVAL, csr.mtval),
        (CSR_STVAL, csr.stval),
        (CSR_MTVEC, csr.mtvec),
        (CSR_STVEC, csr.stvec),
        (CSR_MCAUSE, csr.mcause),
        (CSR_SCAUSE, csr.scause),
        (CSR_SATP, csr.satp),
        (CSR_MIP, csr.mip),
        (CSR_MIE, csr.mie),
        (CSR_MSCRATCH, csr.mscratch),
        (CSR_SSCRATCH, csr.sscratch),
        (CSR_MIDELEG, csr.mideleg),
        (CSR_MEDELEG, csr.medeleg),
    ];

    Some(ContextRestorePlan {
        regs,
        csrs,
        memory_writes: collect_memory_writes(input, original, reset_vector, restore_idx)?,
        target_privilege: csr.privilege_mode,
    })
}

fn nop_skipped_prefix_insts(
    bytes: &mut [u8],
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
    candidate_idx: usize,
    keep_prefix_len: usize,
) -> Option<()> {
    let pc_trace = original.pc_tracker.as_slice();
    let suffix_pcs = pc_trace[candidate_idx..]
        .iter()
        .map(|state| state.value)
        .collect::<HashSet<_>>();
    let mut nopped = HashSet::new();

    for state in &pc_trace[..candidate_idx] {
        if suffix_pcs.contains(&state.value) {
            continue;
        }

        let offset = usize::try_from(state.value.checked_sub(reset_vector)?).ok()?;
        if offset < keep_prefix_len || !nopped.insert(offset) {
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

fn append_context_restore(
    output: &mut Vec<u8>,
    context_pc: u64,
    plan: &ContextRestorePlan,
    target_pc: u64,
) -> Option<()> {
    let use_mret = plan.target_privilege != 3;
    let addr_reg = 1;
    let value_reg = 2;
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
            mstatus_with_mpp(
                plan.csrs.iter().find_map(|&(csr_addr, csr_value)| {
                    (csr_addr == CSR_MSTATUS).then_some(csr_value)
                })?,
                plan.target_privilege,
            ),
            addr_reg,
        );

        let mut continuation_pc = context_pc + (append_inst_count + 1) * 4;
        let mut converged = false;
        for _ in 0..8 {
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

    append_inst_count += append_memory_restore(output, &plan.memory_writes, addr_reg, value_reg);

    for &(reg, value) in plan
        .regs
        .iter()
        .filter(|(reg, _)| *reg != addr_reg && *reg != value_reg)
    {
        append_inst_count += append_load_u64(output, reg, value);
    }

    for &(reg, value) in plan
        .regs
        .iter()
        .filter(|(reg, _)| *reg == addr_reg || *reg == value_reg)
    {
        append_inst_count += append_load_u64(output, reg, value);
    }

    let jal_pc = context_pc + append_inst_count * 4;
    let jmp = encode_jmp(jal_pc, target_pc, false, None)?;
    output.extend_from_slice(&jmp);

    Some(())
}

fn try_strip_prefix_at(
    input: &[u8],
    candidate_idx: usize,
    candidate_pc: u64,
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    let entry_pc = reset_vector;
    let context_pc = reset_vector + input.len() as u64;
    let entry_jmp = encode_jmp(entry_pc, context_pc, true, None)?;
    if input.len() < entry_jmp.len() {
        return None;
    }

    let restore_idx = candidate_idx - 1;
    let restore_plan = collect_context_restore_plan(original, input, restore_idx, reset_vector)?;

    let mut bytes = input.to_vec();
    bytes[0..entry_jmp.len()].copy_from_slice(&entry_jmp);
    nop_skipped_prefix_insts(
        &mut bytes,
        input,
        original,
        reset_vector,
        candidate_idx,
        entry_jmp.len(),
    )?;

    let context_start = bytes.len();
    append_context_restore(&mut bytes, context_pc, &restore_plan, candidate_pc)?;
    let context_inst_count = (bytes.len() - context_start) / 4;

    let suffix_inst_count = original.len().saturating_sub(candidate_idx);
    let max_inst = entry_jmp.len() / 4 + context_inst_count + suffix_inst_count;

    validate_exact_trace(BytesInput::from(bytes), original, max_inst)
}

pub fn strip_irrelevant_prefix(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    println!("Stripping irrelevant prefix...");
    let pc_trace = &original.pc_tracker;
    assert!(pc_trace.len() > 0);

    let jmp_len =
        encode_jmp(reset_vector, reset_vector + input.len() as u64, true, None)?.len() as u64;
    let candidates: Vec<(usize, u64)> = first_dynamic_entries(pc_trace)
        .into_iter()
        .filter(|(_, pc)| reset_vector + jmp_len < *pc)
        .collect();

    let mut low = 0usize;
    let mut high = candidates.len();
    let mut best = None;

    while low < high {
        let mid = low + (high - low) / 2;
        let (candidate_idx, candidate_pc) = candidates[mid];
        match try_strip_prefix_at(input, candidate_idx, candidate_pc, original, reset_vector) {
            Some((candidate, trackers)) => {
                best = Some((candidate, trackers));
                low = mid + 1;
            }
            None => {
                high = mid;
            }
        }
    }

    best
}
