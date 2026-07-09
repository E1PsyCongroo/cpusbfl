mod block;
mod bugloc;
mod coverage;
mod elf;
mod feedback;
mod fuzzer;
mod harness;
mod inst;
mod mutator;
mod observer;
mod reduce;
mod similarity;
mod spectrum;
mod state_tracker;
mod utils;

use std::io::Write;

use clap::Parser;

fn parse_distance_weight(s: &str) -> Result<f64, String> {
    let value: f64 = s.parse::<f64>().map_err(|e| e.to_string())?;

    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{value} is not in range [0, 1]"))
    }
}

#[derive(Parser, Default, Debug)]
struct Arguments {
    // Fuzzer options
    #[clap(default_value_t = false, short, long)]
    fuzzing: bool,
    #[clap(default_value_t = String::from("llvm.branch"), short, long)]
    coverage: String,
    #[clap(default_value_t = String::from("PCState,ArchIntRegState,CSRState"), short, long)]
    state: String,
    #[clap(default_value_t = false, long)]
    base_mutator: bool,
    #[clap(default_value_t = false, short, long)]
    reduce: bool,
    #[clap(default_value_t = false, long)]
    save_reduce: bool,
    #[clap(default_value_t = false, long)]
    save_trace: bool,
    #[clap(default_value_t = 100, long)]
    max_iters: u64,
    #[clap(default_value_t = 10, long)]
    max_run_timeout: u64,
    #[clap(default_value_t = 20, long)]
    tracker_window_size: u64,
    #[clap(default_value_t = 20, long)]
    mutator_window_size: u64,
    #[clap(default_value_t = mutator::lastwindow_mutator::MutationStrategy::Uniform, value_enum, long)]
    mutator_weight_strategy: mutator::lastwindow_mutator::MutationStrategy,
    #[clap(default_value_t = 0.5f64, value_parser = parse_distance_weight, long)]
    cover_distance_weight: f64,
    #[clap(default_value_t = String::from("./corpus"), long)]
    corpus_input: String,
    #[clap(long)]
    output: Option<String>,
    // SBFL options
    #[clap(default_value_t = 10, long)]
    top_pass: u64,
    #[clap(default_value_t = fuzzer::Selection::Sort, long, value_enum)]
    selection: fuzzer::Selection,
    #[clap(default_value_t = 10, long)]
    top_sus: u64,
    #[clap(long)]
    rtl_path: Option<String>,
    #[clap(long, value_delimiter = ',')]
    include_paths: Option<Vec<String>>,
    #[clap(long)]
    top_module: Option<String>,
    #[clap(long)]
    top_scope: Option<String>,
    #[clap(default_value_t = spectrum::matrix::SpectrumMetric::Ochiai, long, value_enum)]
    metric: spectrum::matrix::SpectrumMetric,
    // Run options
    #[clap(default_value_t = 1, long)]
    repeat: usize,
    #[clap(default_value_t = false, long)]
    auto_exit: bool,
    extra_args: Vec<String>,
}

#[unsafe(no_mangle)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Arguments::parse();

    let mut workloads: Vec<String> = Vec::new();
    let mut emu_args: Vec<String> = Vec::new();

    let mut is_emu = false;
    for arg in args.extra_args {
        if arg.starts_with("-") {
            is_emu = true;
        }

        if is_emu {
            emu_args.push(arg);
        } else {
            workloads.push(arg);
        }
    }

    harness::set_sim_env(args.coverage, args.state, emu_args);

    if !workloads.is_empty() {
        let workloads_display = workloads.join(", ");
        let emu_args_display = harness::SIM_ARGS
            .get()
            .unwrap()
            .lock()
            .expect("SIM_ARGS poisoned mutex")
            .join(", ");

        for idx in 0..args.repeat {
            let ret = harness::sim_run_multiple(&workloads, true, false, args.auto_exit);
            if ret != 0 {
                if args.auto_exit {
                    return Err(format!(
                        "workload exited with non-zero status: ret={ret}, \
                 repeat={}/{}, workloads=[{}], emu_args=[{}]",
                        idx + 1,
                        args.repeat,
                        workloads_display,
                        emu_args_display,
                    )
                    .into());
                }
            }
            coverage::all_cover_display();
        }
    }

    if args.fuzzing {
        let sbfl_start_time = utils::process_cpu_time_now()?;
        let input_case = utils::load_initial_case(&args.corpus_input)?;
        let init_case = if args.reduce {
            harness::sim_run_with_trackers(&input_case);
            let original_trackers = state_tracker::trackers().clone();
            let reducing_start_time = utils::process_cpu_time_now()?;
            let reduced_case = reduce::reduce_fault_case(
                input_case,
                original_trackers,
                args.save_reduce,
                &args.output,
            );
            let reducing_end_time = utils::process_cpu_time_now()?;
            let reducing_elapsed = reducing_end_time
                .checked_sub(reducing_start_time)
                .unwrap_or_default();
            log::info!("Reducing process CPU time = {reducing_elapsed:?}");
            if let Some(output_dir) = args.output.as_ref() {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(
                        std::path::PathBuf::from(format!("{output_dir}/reducing_time.txt"))
                            .as_path(),
                    )?
                    .write_fmt(format_args!("{reducing_elapsed:?}"))?;
            }
            reduced_case
        } else {
            input_case
        };

        fuzzer::run_fuzzer(
            args.base_mutator,
            args.max_iters,
            args.max_run_timeout,
            args.tracker_window_size,
            args.mutator_weight_strategy,
            args.mutator_window_size,
            args.cover_distance_weight,
            args.top_pass,
            args.selection,
            args.save_trace,
            &init_case,
            &args.output,
        )
        .and_then(|passed_cov| {
            bugloc::report_result(
                args.top_sus,
                args.metric,
                &passed_cov,
                &args.rtl_path,
                &args.include_paths,
                &args.top_module,
                &args.top_scope,
                &args.output,
            )
        })?;
        let sbfl_end_time = utils::process_cpu_time_now()?;
        let sbfl_elapsed = sbfl_end_time
            .checked_sub(sbfl_start_time)
            .unwrap_or_default();
        log::info!("SBFL process CPU time = {sbfl_elapsed:?}");
        if let Some(output_dir) = args.output.as_ref() {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(std::path::PathBuf::from(format!("{output_dir}/sbfl_time.txt")).as_path())?
                .write_fmt(format_args!("{sbfl_elapsed:?}"))?;
        }
    }

    Ok(())
}
