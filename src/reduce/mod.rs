mod nop;
mod strip_prefix;
mod strip_suffix;
mod trim;

use libafl::prelude::*;

use crate::harness::{sim_run_with_trackers, sim_with_max_inst};
use crate::monitor::*;
use crate::state_tracker::*;
use nop::nop_unexecuted_insts;
use strip_prefix::strip_irrelevant_prefix;
use strip_suffix::strip_irrelevant_suffix;
use trim::trim_after_max_pc;

const JAL_X0_OPCODE: u32 = 0x6f;

fn riscv_inst_len(first_halfword: u16) -> usize {
    if first_halfword & 0b11 == 0b11 { 4 } else { 2 }
}

pub fn inst_len_at(input: &[u8], offset: usize) -> usize {
    assert!(offset + 2 <= input.len());

    let halfword = u16::from_le_bytes([input[offset], input[offset + 1]]);
    let inst_len = riscv_inst_len(halfword);
    assert!(offset + inst_len <= input.len());

    inst_len
}

pub fn pc_to_offset(input: &[u8], reset_vector: u64, pc: u64) -> usize {
    assert!(pc >= reset_vector);
    let offset = usize::try_from(pc - reset_vector).unwrap();
    assert!(offset < input.len());

    offset
}

fn run_and_collect(input: &BytesInput, max_inst: usize) -> (ExitKind, StateTrackers) {
    sim_with_max_inst(max_inst, || {
        let exit_kind = sim_run_with_trackers(input);
        let state_trackers = trackers().clone();
        (exit_kind, state_trackers)
    })
}

fn is_same_failure_site(
    exit_kind: ExitKind,
    candidate: &StateTrackers,
    original: &StateTrackers,
) -> bool {
    if !matches!(exit_kind, ExitKind::Crash) {
        return false;
    }

    let Some(candidate_pc) = candidate.pc_tracker.as_slice().last() else {
        return false;
    };
    let Some(original_pc) = original.pc_tracker.as_slice().last() else {
        return false;
    };

    candidate_pc == original_pc
}

pub fn validate_exact_trace(
    input: BytesInput,
    original: &StateTrackers,
    max_inst: usize,
) -> Option<(BytesInput, StateTrackers)> {
    let (exit_kind, candidate) = run_and_collect(&input, max_inst);
    if is_same_failure_site(exit_kind, &candidate, original) {
        Some((input, candidate))
    } else {
        None
    }
}

pub fn encode_jal_x0(from_pc: u64, to_pc: u64) -> Option<[u8; 4]> {
    let offset = i128::from(to_pc) - i128::from(from_pc);
    assert!(offset % 2 == 0);
    if offset < -(1 << 20) || offset > (1 << 20) - 2 {
        return None;
    }

    let imm = (offset as i32) as u32;
    let inst = (((imm >> 20) & 0x1) << 31)
        | (((imm >> 1) & 0x3ff) << 21)
        | (((imm >> 11) & 0x1) << 20)
        | (((imm >> 12) & 0xff) << 12)
        | JAL_X0_OPCODE;

    Some(inst.to_le_bytes())
}

pub(crate) fn reduce_fault_case(
    input: &BytesInput,
    original: &StateTrackers,
    reset_vector: u64,
    save_reduce: bool,
    output_dir: &Option<String>,
) -> BytesInput {
    assert!(original.len() > 0);
    let output_dir = output_dir.as_deref();

    let trimmed_bytes = match trim_after_max_pc(input.mutator_bytes(), &original, reset_vector) {
        Some((bytes, _)) => {
            println!("Trim case after max pc successed");
            if save_reduce && output_dir.is_some() {
                store_testcase(
                    &bytes,
                    None,
                    output_dir.unwrap(),
                    Some("init_timmed".to_string()),
                );
            }
            bytes
        }
        None => input.to_owned(),
    };

    let nopped_bytes = match nop_unexecuted_insts(trimmed_bytes.mutator_bytes(), &original, reset_vector) {
        Some((bytes, _)) => {
            println!("Nop unexecuted insts successed");
            if save_reduce && output_dir.is_some() {
                store_testcase(
                    &bytes,
                    None,
                    output_dir.unwrap(),
                    Some("init_nopped".to_string()),
                );
            }
            bytes
        }
        None => trimmed_bytes.to_owned(),
    };

    let (striped_suffix_bytes, striped_suffix_trackers) =
        match strip_irrelevant_suffix(nopped_bytes.mutator_bytes(), original, reset_vector) {
            Some((bytes, trackers)) => {
                println!("Strip irrelevant suffix insts successed");
                if save_reduce && output_dir.is_some() {
                    store_testcase(
                        &bytes,
                        None,
                        output_dir.unwrap(),
                        Some("init_striped_suffix".to_string()),
                    );
                }
                (bytes, trackers)
            }
            None => (nopped_bytes.to_owned(), original.to_owned()),
        };

    // let striped_prefix_bytes =
    //     match strip_irrelevant_prefix(striped_suffix_bytes.mutator_bytes(), &striped_suffix_trackers, reset_vector) {
    //         Some((bytes, _)) => {
    //             println!("Strip irrelevant prefix insts successed");
    //             if save_reduce && output_dir.is_some() {
    //                 store_testcase(
    //                     &bytes,
    //                     None,
    //                     output_dir.unwrap(),
    //                     Some("init_striped_prefix".to_string()),
    //                 );
    //             }
    //             bytes
    //         }
    //         None => striped_suffix_bytes.to_owned(),
    //     };

    // striped_prefix_bytes

    striped_suffix_bytes
}
