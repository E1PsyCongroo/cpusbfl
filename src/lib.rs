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
mod scheduler;
mod similarity;
mod spectrum;
mod state_tracker;
mod utils;

use std::io::Write;

use clap::{Args, Parser, Subcommand};

fn parse_weight(s: &str) -> Result<f64, String> {
    let value: f64 = s.parse::<f64>().map_err(|e| e.to_string())?;

    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{value} is not in range [0, 1]"))
    }
}

#[derive(Args, Debug)]
struct RTLArgs {
    #[arg(long, requires_all = ["top_module", "top_scope"])]
    rtl_path: Option<String>,

    #[arg(long, value_delimiter = ',', requires = "rtl_path")]
    include_paths: Option<Vec<String>>,

    #[arg(long, requires = "rtl_path")]
    top_module: Option<String>,

    #[arg(long, requires = "rtl_path")]
    top_scope: Option<String>,
}

#[derive(Args, Debug)]
struct SBFLArgs {
    #[arg(default_value_t = String::from("./corpus"), long)]
    corpus_input: String,
    #[arg(long)]
    output: Option<String>,

    #[arg(default_value_t = false, short = 'r', long, alias = "reduce")]
    reduce_insts: bool,
    #[arg(default_value_t = false, long)]
    reduce_cover: bool,
    #[arg(default_value_t = false, long)]
    save_reduce: bool,
    #[arg(default_value_t = false, long)]
    save_trace: bool,
    #[arg(default_value_t = 100, long)]
    max_iters: u64,
    #[arg(default_value_t = 10, long)]
    max_run_timeout: u64,
    #[arg(default_value_t = 20, long)]
    tracker_window_size: u64,
    #[arg(default_value_t = 0.5f64, value_parser = parse_weight, long)]
    cover_distance_weight: f64,

    #[arg(default_value_t = 10, long)]
    top_pass: u64,
    #[arg(default_value_t = fuzzer::Selection::Sort, long, value_enum)]
    selection: fuzzer::Selection,
    #[arg(default_value_t = 10, long)]
    top_sus: u64,
    #[command(flatten)]
    rtl: RTLArgs,
    #[arg(default_value_t = spectrum::SpectrumMetric::Ochiai, long, value_enum)]
    metric: spectrum::SpectrumMetric,
}

#[derive(Subcommand, Debug)]
enum SBFLMode {
    PSBFL {
        #[arg(default_value_t = 20, long)]
        mutator_window_size: u64,
        #[arg(default_value_t = mutator::PSBFLMutationStrategy::Uniform, value_enum, long)]
        mutator_weight_strategy: mutator::PSBFLMutationStrategy,
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    Random {
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    WitHW {
        #[arg(default_value_t = 50, long)]
        max_corpus_size: usize,
        #[arg(default_value_t = 0.2f64, value_parser = parse_weight, long)]
        init_seed_rate: f64,
        #[arg(default_value_t = 0.2f64, value_parser = parse_weight, long)]
        mutate_rate: f64,
        #[arg(default_value_t = 0.1f64, value_parser = parse_weight, long)]
        priority_alpha: f64,
        #[arg(default_value_t = 5.0f64, long)]
        failed_reward: f64,
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum Command {
    Workload {
        #[arg(default_value_t = 1, long)]
        repeat: usize,
        #[arg(default_value_t = false, long)]
        auto_exit: bool,
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
    SBFL {
        #[command(subcommand)]
        mode: SBFLMode,
        #[command(flatten)]
        sbfl_args: SBFLArgs,
    },
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[arg(default_value_t = String::from("verilator.branch,verilator.line"), short, long)]
    coverage: String,
    #[arg(default_value_t = String::from("PCState,ArchIntRegState,CSRState"), short, long)]
    state: String,
}

fn sbfl_extra_args(mode: &SBFLMode) -> Vec<String> {
    match mode {
        SBFLMode::PSBFL { extra_args, .. }
        | SBFLMode::Random { extra_args }
        | SBFLMode::WitHW { extra_args, .. } => extra_args.clone(),
    }
}

fn split_extra_args(extra_args: Vec<String>) -> (Vec<String>, Vec<String>) {
    let split_index = extra_args
        .iter()
        .position(|arg| arg.starts_with('-'))
        .unwrap_or(extra_args.len());

    let mut workloads = extra_args;
    let emu_args = workloads.split_off(split_index);

    (workloads, emu_args)
}

fn _main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let Cli {
        command,
        coverage,
        state,
    } = Cli::parse();

    let mut workloads: Vec<String> = Vec::new();
    let mut emu_args: Vec<String> = Vec::new();

    match command {
        Command::Workload {
            repeat,
            auto_exit,
            extra_args,
        } => {
            let (workloads, emu_args) = split_extra_args(extra_args);
            harness::set_sim_env(coverage, state, emu_args);

            if !workloads.is_empty() {
                let workloads_display = workloads.join(", ");
                let emu_args_display = harness::SIM_ARGS
                    .get()
                    .unwrap()
                    .lock()
                    .expect("SIM_ARGS poisoned mutex")
                    .join(", ");

                for idx in 0..repeat {
                    let ret = harness::sim_run_multiple(&workloads, true, false, auto_exit);
                    if ret != 0 {
                        if auto_exit {
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
                    }
                    coverage::all_cover_display();
                }
            }
        }
        Command::SBFL { mode, sbfl_args } => {
            let extra_args = sbfl_extra_args(&mode);
            let (_, emu_args) = split_extra_args(extra_args);
            harness::set_sim_env(coverage, state, emu_args);

            if let Some(output_dir) = sbfl_args.output.as_ref() {
                std::fs::create_dir_all(&output_dir)?;
            }

            let sbfl_start_time = utils::process_cpu_time_now()?;
            let input_case = utils::load_initial_case(&sbfl_args.corpus_input)?;
            let init_case = if sbfl_args.reduce_insts {
                harness::fuzz_harness(&input_case);
                let original_trackers = state_tracker::trackers().clone();
                let reducing_start_time = utils::process_cpu_time_now()?;
                let reduced_case = reduce::reduce_fault_case(
                    input_case,
                    original_trackers,
                    sbfl_args.save_reduce,
                    &sbfl_args.output,
                );
                let reducing_end_time = utils::process_cpu_time_now()?;
                let reducing_elapsed = reducing_end_time
                    .checked_sub(reducing_start_time)
                    .unwrap_or_default();
                log::info!("Reducing process CPU time = {reducing_elapsed:?}");
                if let Some(output_dir) = sbfl_args.output.as_ref() {
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

            let passed_cov = match mode {
                SBFLMode::PSBFL {
                    mutator_weight_strategy,
                    mutator_window_size,
                    ..
                } => {
                    let scheduler = libafl::schedulers::QueueScheduler::new();
                    fuzzer::run_fuzzer(
                        &init_case,
                        &sbfl_args.output,
                        sbfl_args.max_iters,
                        sbfl_args.max_run_timeout,
                        sbfl_args.tracker_window_size,
                        sbfl_args.cover_distance_weight,
                        sbfl_args.top_pass,
                        sbfl_args.selection,
                        sbfl_args.reduce_cover,
                        sbfl_args.save_trace,
                        scheduler,
                        |metadata| {
                            crate::mutator::PSBFLMutator::new(
                                &init_case,
                                &metadata.state_trackers.pc_tracker,
                                mutator_weight_strategy,
                                mutator_window_size,
                            )
                        },
                    )?
                }
                SBFLMode::Random { .. } => {
                    let scheduler = libafl::schedulers::QueueScheduler::new();
                    fuzzer::run_fuzzer(
                        &init_case,
                        &sbfl_args.output,
                        sbfl_args.max_iters,
                        sbfl_args.max_run_timeout,
                        sbfl_args.tracker_window_size,
                        sbfl_args.cover_distance_weight,
                        sbfl_args.top_pass,
                        sbfl_args.selection,
                        sbfl_args.reduce_cover,
                        sbfl_args.save_trace,
                        scheduler,
                        |_| {
                            crate::mutator::ELFHavocScheduledMutator::new(
                                libafl::mutators::havoc_mutations(),
                                &init_case,
                            )
                        },
                    )?
                }
                SBFLMode::WitHW {
                    max_corpus_size,
                    init_seed_rate,
                    mutate_rate,
                    priority_alpha,
                    failed_reward,
                    ..
                } => {
                    let scheduler = crate::scheduler::WitScheduler::new(
                        init_seed_rate,
                        sbfl_args.cover_distance_weight,
                    );
                    fuzzer::run_fuzzer(
                        &init_case,
                        &sbfl_args.output,
                        sbfl_args.max_iters,
                        sbfl_args.max_run_timeout,
                        sbfl_args.tracker_window_size,
                        sbfl_args.cover_distance_weight,
                        sbfl_args.top_pass,
                        sbfl_args.selection,
                        sbfl_args.reduce_cover,
                        sbfl_args.save_trace,
                        scheduler,
                        |metadata| {
                            let mutator = crate::mutator::WitHWMutator::new(
                                &init_case,
                                &metadata.state_trackers.pc_tracker,
                                sbfl_args.tracker_window_size,
                                sbfl_args.cover_distance_weight,
                                mutate_rate,
                                priority_alpha,
                                failed_reward,
                            )?;
                            Ok(crate::mutator::BoundedMutator::new(
                                mutator,
                                max_corpus_size,
                                sbfl_args.cover_distance_weight,
                            ))
                        },
                    )?
                }
            };

            bugloc::report_result(
                sbfl_args.top_sus,
                sbfl_args.metric,
                passed_cov,
                &sbfl_args.rtl.rtl_path,
                &sbfl_args.rtl.include_paths,
                &sbfl_args.rtl.top_module,
                &sbfl_args.rtl.top_scope,
                &sbfl_args.output,
            )?;
            let sbfl_end_time = utils::process_cpu_time_now()?;
            let sbfl_elapsed = sbfl_end_time
                .checked_sub(sbfl_start_time)
                .unwrap_or_default();
            log::info!("SBFL process CPU time = {sbfl_elapsed:?}");
            if let Some(output_dir) = sbfl_args.output.as_ref() {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(
                        std::path::PathBuf::from(format!("{output_dir}/sbfl_time.txt")).as_path(),
                    )?
                    .write_fmt(format_args!("{sbfl_elapsed:?}"))?;
            }
        }
    }

    Ok(())
}

#[unsafe(no_mangle)]
fn main() {
    _main().expect("sbfl")
}
