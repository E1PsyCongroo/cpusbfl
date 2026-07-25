use std::{
    ffi::{CString, c_char, c_int, c_uint, c_void},
    fs::File,
    io::{self, Write},
    os::fd::AsRawFd,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use libafl::prelude::*;
use tempfile::Builder;

use crate::coverage::*;
use crate::state_tracker::*;

unsafe extern "C" {
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

    pub fn update_stats_state(state_tracker: *mut c_void);

    pub fn set_state_feedback(name: *const c_char);
}

pub(crate) static SIM_ARGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static SAVE_INTERMEDIATE: AtomicBool = AtomicBool::new(false);

struct SimOutputSilencer {
    saved_stdout: c_int,
    saved_stderr: c_int,
}

impl SimOutputSilencer {
    fn new() -> io::Result<Self> {
        flush_process_output();

        let null = File::options().write(true).open("/dev/null")?;
        let saved_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved_stdout < 0 {
            return Err(io::Error::last_os_error());
        }

        let saved_stderr = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved_stderr < 0 {
            unsafe {
                libc::close(saved_stdout);
            }
            return Err(io::Error::last_os_error());
        }

        if unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) } < 0 {
            unsafe {
                libc::close(saved_stdout);
                libc::close(saved_stderr);
            }
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::dup2(null.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
            unsafe {
                libc::dup2(saved_stdout, libc::STDOUT_FILENO);
                libc::close(saved_stdout);
                libc::close(saved_stderr);
            }
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            saved_stdout,
            saved_stderr,
        })
    }
}

impl Drop for SimOutputSilencer {
    fn drop(&mut self) {
        flush_process_output();
        unsafe {
            libc::dup2(self.saved_stdout, libc::STDOUT_FILENO);
            libc::dup2(self.saved_stderr, libc::STDERR_FILENO);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
        }
    }
}

fn flush_process_output() {
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    unsafe {
        libc::fflush(std::ptr::null_mut());
    }
}

fn sim_run(workload: &str, update_cover: bool, update_tracker: bool) -> i32 {
    // prepare the simulation arguments in Vec<String> format
    let mut sim_args: Vec<String> = vec!["emu".to_string(), "-E".to_string(), workload.to_string()]
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

    if update_cover {
        all_cover_update();
    }
    if update_tracker {
        all_tracker_update();
    }

    ret
}

fn sim_run_from_memory(input: &BytesInput, update_cover: bool, update_tracker: bool) -> i32 {
    let bytes = input.mutator_bytes();

    let mut tmp = Builder::new()
        .prefix("sbfl-simrun-")
        .tempfile()
        .expect("failed to create temporary sbfl simrun file");

    tmp.write_all(bytes)
        .expect("failed to write input to temporary sbfl simrun file");

    tmp.flush()
        .expect("failed to flush temporary sbfl simrun file");

    let path = tmp
        .path()
        .to_str()
        .expect("temporary sbfl simrun file path is not valid UTF-8");

    sim_run(path, update_cover, update_tracker)
}

pub(crate) fn sim_run_multiple(
    workloads: &Vec<String>,
    update_cover: bool,
    update_tracker: bool,
    auto_exit: bool,
) -> i32 {
    let mut ret = 0;
    for workload in workloads.iter() {
        ret = sim_run(workload, update_cover, update_tracker);
        if ret != 0 {
            log::info!("{} exits abnormally with return code: {}", workload, ret);
            if auto_exit {
                break;
            }
        }
    }
    return ret;
}

pub(crate) fn sim_with_max_inst<T>(max_inst: usize, f: impl FnOnce() -> T) -> T {
    let original_args = SIM_ARGS
        .get()
        .expect("SIM_ARGS not initialized")
        .lock()
        .expect("poisoned mutex")
        .clone();
    SIM_ARGS
        .get()
        .expect("SIM_ARGS not initialized")
        .lock()
        .expect("poisoned mutex")
        .extend(vec!["-I".to_string(), max_inst.to_string()].into_iter());

    let ret = f();

    SIM_ARGS
        .get()
        .expect("SIM_ARGS not initialized")
        .lock()
        .expect("poisoned mutex")
        .clone_from(&original_args);

    ret
}

pub(crate) fn fuzz_harness(input: &BytesInput) -> ExitKind {
    let ret = if SAVE_INTERMEDIATE.load(Ordering::Relaxed) {
        sim_run_from_memory(input, true, true)
    } else {
        let _silencer =
            SimOutputSilencer::new().expect("failed to redirect simulation output to /dev/null");
        sim_run_from_memory(input, true, true)
    };

    // get coverage
    for cover_name in cover_names() {
        cover_display(&cover_name);
    }
    io::stdout().flush().unwrap();

    if ret != 0 {
        ExitKind::Crash
    } else {
        ExitKind::Ok
    }
}

pub(crate) fn set_sim_env(
    cover_names: String,
    state_names: String,
    emu_args: Vec<String>,
    save_intermediate: bool,
) {
    let _ = SIM_ARGS.set(Mutex::new(emu_args));
    SAVE_INTERMEDIATE.store(save_intermediate, Ordering::Relaxed);

    cover_init(
        cover_names
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect(),
    );

    state_tracker_init(
        state_names
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect(),
    );
}
