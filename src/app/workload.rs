use crate::{app::AppResult, cli::split_extra_args, coverage, harness};

pub(super) fn run(
    coverage_names: String,
    state_names: String,
    repeat: usize,
    auto_exit: bool,
    extra_args: Vec<String>,
) -> AppResult {
    let (workloads, emu_args) = split_extra_args(extra_args);
    harness::set_sim_env(coverage_names, state_names, emu_args);

    if workloads.is_empty() {
        return Ok(());
    }

    let workloads_display = workloads.join(", ");
    let emu_args_display = harness::SIM_ARGS
        .get()
        .unwrap()
        .lock()
        .expect("SIM_ARGS poisoned mutex")
        .join(", ");

    for idx in 0..repeat {
        let ret = harness::sim_run_multiple(&workloads, true, false, auto_exit);
        if ret != 0 && auto_exit {
            return Err(format!(
                "workload exited with non-zero status: ret={ret}, \
                 repeat={}/{}, workloads=[{}], emu_args=[{}]",
                idx + 1,
                repeat,
                workloads_display,
                emu_args_display,
            )
            .into());
        }
        coverage::all_cover_display();
    }

    Ok(())
}
