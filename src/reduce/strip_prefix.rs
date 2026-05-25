use std::collections::HashSet;

use libafl::prelude::*;

use crate::reduce::*;
use crate::state_tracker::*;

const ADDI_OPCODE: u32 = 0x13;
const SYSTEM_OPCODE: u32 = 0x73;
const MRET: [u8; 4] = 0x3020_0073_u32.to_le_bytes();

const CSR_SSTATUS: u16 = 0x100;
const CSR_STVEC: u16 = 0x105;
const CSR_SEPC: u16 = 0x141;
const CSR_SCAUSE: u16 = 0x142;
const CSR_STVAL: u16 = 0x143;
const CSR_SSCRATCH: u16 = 0x140;
const CSR_SATP: u16 = 0x180;
const CSR_MSTATUS: u16 = 0x300;
const CSR_MEDELEG: u16 = 0x302;
const CSR_MIDELEG: u16 = 0x303;
const CSR_MIE: u16 = 0x304;
const CSR_MTVEC: u16 = 0x305;
const CSR_MSCRATCH: u16 = 0x340;
const CSR_MEPC: u16 = 0x341;
const CSR_MCAUSE: u16 = 0x342;
const CSR_MTVAL: u16 = 0x343;
const CSR_MIP: u16 = 0x344;

#[derive(Clone, Copy)]
struct CsrRestore {
    csr: u16,
    value: u64,
}

struct ContextRestorePlan {
    changed_regs: Vec<(u8, u64)>,
    changed_csrs: Vec<CsrRestore>,
    privilege_changed: bool,
    target_privilege: u64,
    target_mstatus: u64,
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

fn push_inst(output: &mut Vec<u8>, inst: u32) {
    output.extend_from_slice(&inst.to_le_bytes());
}

fn encode_addi(rd: u8, rs1: u8, imm: i16) -> u32 {
    assert!((-2048..=2047).contains(&imm));
    (((imm as i32 as u32) & 0xfff) << 20) | ((rs1 as u32) << 15) | ((rd as u32) << 7) | ADDI_OPCODE
}

fn encode_slli(rd: u8, rs1: u8, shamt: u8) -> u32 {
    assert!(shamt < 64);
    ((shamt as u32) << 20) | ((rs1 as u32) << 15) | (0b001 << 12) | ((rd as u32) << 7) | ADDI_OPCODE
}

fn encode_csrrw(csr: u16, rs1: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b001 << 12) | SYSTEM_OPCODE
}

fn append_load_u64(output: &mut Vec<u8>, rd: u8, value: u64) {
    push_inst(output, encode_addi(rd, 0, 0));
    if value == 0 {
        return;
    }

    let highest_bit = 63 - value.leading_zeros();
    for bit in (0..=highest_bit).rev() {
        push_inst(output, encode_slli(rd, rd, 1));
        if (value >> bit) & 1 == 1 {
            push_inst(output, encode_addi(rd, rd, 1));
        }
    }
}

fn append_write_csr(output: &mut Vec<u8>, csr: u16, value: u64, scratch_reg: u8) {
    append_load_u64(output, scratch_reg, value);
    push_inst(output, encode_csrrw(csr, scratch_reg));
}

fn mstatus_with_mpp(mstatus: u64, privilege_mode: u64) -> u64 {
    let mpp = match privilege_mode {
        0 | 1 | 3 => privilege_mode,
        _ => 3,
    };

    (mstatus & !(0b11 << 11)) | (mpp << 11)
}

fn reg_changed_before(
    tracker: &StateTracker<ArchIntRegState>,
    restore_idx: usize,
    reg: usize,
) -> bool {
    for idx in 1..=restore_idx {
        let before = tracker.as_slice()[idx - 1].value[reg];
        let after = tracker.as_slice()[idx].value[reg];
        if before != after {
            return true;
        }
    }

    false
}

fn csr_field_changed_before(
    tracker: &StateTracker<CSRState>,
    restore_idx: usize,
    get: impl Fn(&CSRState) -> u64,
) -> bool {
    for idx in 1..=restore_idx {
        let before = get(&tracker.as_slice()[idx - 1]);
        let after = get(&tracker.as_slice()[idx]);
        if before != after {
            return true;
        }
    }

    false
}

fn collect_context_restore_plan(
    original: &StateTrackers,
    restore_idx: usize,
) -> Option<ContextRestorePlan> {
    let arch = original.arch_int_reg_tracker.as_slice().get(restore_idx)?;
    let csr = original.csr_tracker.as_slice().get(restore_idx)?;

    let mut changed_regs = Vec::new();
    for reg in 1..32 {
        if reg_changed_before(&original.arch_int_reg_tracker, restore_idx, reg) {
            changed_regs.push((reg as u8, arch.value[reg]));
        }
    }

    let csr_fields: [(u16, fn(&CSRState) -> u64); 17] = [
        (CSR_MSTATUS, |s| s.mstatus),
        (CSR_SSTATUS, |s| s.sstatus),
        (CSR_MEPC, |s| s.mepc),
        (CSR_SEPC, |s| s.sepc),
        (CSR_MTVAL, |s| s.mtval),
        (CSR_STVAL, |s| s.stval),
        (CSR_MTVEC, |s| s.mtvec),
        (CSR_STVEC, |s| s.stvec),
        (CSR_MCAUSE, |s| s.mcause),
        (CSR_SCAUSE, |s| s.scause),
        (CSR_SATP, |s| s.satp),
        (CSR_MIP, |s| s.mip),
        (CSR_MIE, |s| s.mie),
        (CSR_MSCRATCH, |s| s.mscratch),
        (CSR_SSCRATCH, |s| s.sscratch),
        (CSR_MIDELEG, |s| s.mideleg),
        (CSR_MEDELEG, |s| s.medeleg),
    ];
    let mut changed_csrs = Vec::new();
    for (csr_num, get) in csr_fields {
        if csr_field_changed_before(&original.csr_tracker, restore_idx, get) {
            changed_csrs.push(CsrRestore {
                csr: csr_num,
                value: get(csr),
            });
        }
    }

    let privilege_changed =
        csr_field_changed_before(&original.csr_tracker, restore_idx, |s| s.privilege_mode);

    Some(ContextRestorePlan {
        changed_regs,
        changed_csrs,
        privilege_changed,
        target_privilege: csr.privilege_mode,
        target_mstatus: csr.mstatus,
    })
}

fn append_context_restore(
    output: &mut Vec<u8>,
    context_pc: u64,
    plan: &ContextRestorePlan,
    target_pc: u64,
) -> Option<()> {
    let context_start_len = output.len();
    let csr_restore_uses_scratch = !plan.changed_csrs.is_empty() || plan.privilege_changed;
    let scratch_reg = if csr_restore_uses_scratch {
        Some(plan.changed_regs.last()?.0)
    } else {
        None
    };

    for restore in &plan.changed_csrs {
        if plan.privilege_changed && restore.csr == CSR_MSTATUS {
            continue;
        }
        if plan.privilege_changed && restore.csr == CSR_MEPC {
            continue;
        }
        append_write_csr(output, restore.csr, restore.value, scratch_reg.unwrap());
    }

    if plan.privilege_changed {
        append_write_csr(
            output,
            CSR_MSTATUS,
            mstatus_with_mpp(plan.target_mstatus, plan.target_privilege),
            scratch_reg.unwrap(),
        );
        append_write_csr(output, CSR_MEPC, target_pc, scratch_reg.unwrap());
    }

    for (reg, value) in plan
        .changed_regs
        .iter()
        .filter(|(reg, _)| Some(*reg) != scratch_reg)
    {
        append_load_u64(output, *reg, *value);
    }

    if let Some(scratch_reg) = scratch_reg {
        let (_, value) = plan
            .changed_regs
            .iter()
            .find(|(reg, _)| *reg == scratch_reg)
            .unwrap();
        append_load_u64(output, scratch_reg, *value);
    }

    if plan.privilege_changed {
        output.extend_from_slice(&MRET);
    } else {
        let jal_pc = context_pc + (output.len() - context_start_len) as u64;
        let jal = encode_jal_x0(jal_pc, target_pc)?;
        output.extend_from_slice(&jal);
    }

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
    let entry_jal = encode_jal_x0(entry_pc, context_pc)?;

    let restore_idx = candidate_idx - 1;
    let restore_plan = collect_context_restore_plan(original, restore_idx)?;

    let mut bytes = input.to_vec();
    bytes[0..4].copy_from_slice(&entry_jal);

    let context_start = bytes.len();
    append_context_restore(&mut bytes, context_pc, &restore_plan, candidate_pc)?;
    let context_inst_count = (bytes.len() - context_start) / 4;

    let suffix_inst_count = original.len().saturating_sub(candidate_idx);
    let max_inst = 1 + context_inst_count + suffix_inst_count;

    validate_exact_trace(BytesInput::from(bytes), original, max_inst)
}

pub fn strip_irrelevant_prefix(
    input: &[u8],
    original: &StateTrackers,
    reset_vector: u64,
) -> Option<(BytesInput, StateTrackers)> {
    let pc_trace = &original.pc_tracker;
    assert!(pc_trace.len() > 0);

    encode_jal_x0(reset_vector, reset_vector + input.len() as u64)?;
    let candidates: Vec<(usize, u64)> = first_dynamic_entries(pc_trace)
        .into_iter()
        .filter(|(_, pc)| reset_vector + 4 < *pc)
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
