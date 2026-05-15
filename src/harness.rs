use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

use libafl::prelude::*;

use crate::coverage::*;
use crate::monitor::store_testcase;
use crate::pc_trace::*;
use crate::state_tracker::*;

unsafe extern "C" {
    pub fn enable_sim_verbose();

    pub fn disable_sim_verbose();

    pub fn sim_main(argc: c_int, argv: *const *const c_char) -> c_int;

    // coverage
    pub fn get_cover_number() -> c_uint;

    pub fn get_cover_point_name(i: usize) -> *const c_char;

    pub fn get_cover_data_size() -> usize;

    pub fn update_stats_cover(data: *mut c_void);

    pub fn display_uncovered_points();

    pub fn set_cover_feedback(name: *const c_char);

    // state
    pub fn get_state_number() -> usize;

    pub fn get_state_size() -> usize;

    pub fn update_stats_state(state_tracker: *mut c_void);

    pub fn set_state_feedback(name: *const c_char);

    // pc trace
    pub fn get_pc_trace_size() -> usize;

    pub fn update_stats_pc_trace(pc_trace: *mut u64);

}

pub(crate) static SIM_ARGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn sim_run(workload: &String) -> i32 {
    // prepare the simulation arguments in Vec<String> format
    let mut sim_args: Vec<String> = vec!["emu".to_string(), "-i".to_string(), workload.to_string()]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let guard = SIM_ARGS
        .get()
        .expect("SIM_ARGS not initialized")
        .lock()
        .unwrap();
    sim_args.extend(guard.iter().cloned());

    // convert the simulation arguments into c_char**
    let sim_args: Vec<_> = sim_args
        .iter()
        .map(|s| CString::new(s.as_bytes()).unwrap())
        .collect();
    let mut p_argv: Vec<_> = sim_args.iter().map(|arg| arg.as_ptr()).collect();
    p_argv.push(std::ptr::null());

    // send simulation arguments to sim_main and get the return code
    let ret = unsafe { sim_main(sim_args.len() as i32, p_argv.as_ptr()) };
    all_cover_update_stats();
    all_cover_accumulate();
    all_tracker_update_stats();

    ret
}

fn sim_run_from_memory(input: &BytesInput) -> i32 {
    // create a workload-in-memory name for the input bytes
    let wim_bytes = input.mutator_bytes();
    let wim_addr = wim_bytes.as_ptr();
    let wim_size = wim_bytes.len() as u64;
    let wim_name = format!("wim@{wim_addr:p}+0x{wim_size:x}");
    // pass the in-memory workload to sim_run
    sim_run(&wim_name)
}

pub(crate) fn sim_run_multiple(workloads: &Vec<String>, auto_exit: bool) -> i32 {
    let mut ret = 0;
    for workload in workloads.iter() {
        ret = sim_run(workload);
        if ret != 0 {
            println!("{} exits abnormally with return code: {}", workload, ret);
            if auto_exit {
                break;
            }
        }
    }
    return ret;
}

pub static mut SAVE_ERRORS: bool = false;

pub(crate) fn fuzz_harness(input: &BytesInput) -> ExitKind {
    let ret = sim_run_from_memory(input);

    // get coverage
    for cover_name in cover_names() {
        cover_display(&cover_name);
    }
    io::stdout().flush().unwrap();

    // save the target testcase into disk
    let do_save = unsafe { SAVE_ERRORS && ret != 0 };
    if do_save {
        store_testcase(input, None, &"errors".to_string(), None);
    }

    if ret != 0 {
        ExitKind::Crash
    } else {
        ExitKind::Ok
    }
}

pub(crate) fn set_sim_env(cover_names: String, verbose: bool, emu_args: Vec<String>) {
    if verbose {
        unsafe { enable_sim_verbose() }
    } else {
        unsafe { disable_sim_verbose() }
    }

    let _ = SIM_ARGS.set(Mutex::new(emu_args));

    cover_init(
        cover_names
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect(),
    );

    state_tracker_init("ArchIntRegState".to_string());

    pc_trace_init();
}
