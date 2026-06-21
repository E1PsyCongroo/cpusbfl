mod block;
mod bugloc;
mod coverage;
mod elf;
mod feedback;
mod fuzzer;
mod harness;
mod monitor;
mod mutator;
mod observer;
mod reduce;
mod similarity;
mod spectrum;
mod state_tracker;

use clap::Parser;

#[derive(Parser, Default, Debug)]
struct Arguments {
    // Fuzzer options
    #[clap(default_value_t = false, short, long)]
    fuzzing: bool,
    #[clap(default_value_t = String::from("llvm.branch"), short, long)]
    coverage: String,
    #[clap(default_value_t = String::from("PCState,ArchIntRegState,CSRState"), short, long)]
    state: String,
    #[clap(default_value_t = false, short, long)]
    reduce: bool,
    #[clap(default_value_t = 100, long)]
    max_iters: u64,
    #[clap(default_value_t = 10, long)]
    max_run_timeout: u64,
    #[clap(default_value_t = String::from("./corpus"), long)]
    corpus_input: String,
    #[clap(long)]
    output: Option<String>,
    #[clap(default_value_t = false, long)]
    save_reduce: bool,
    // SBFL options
    #[clap(default_value_t = 10, long)]
    top_pass: u64,
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
    #[arg(long)]
    ground_truth: Option<String>,
    // Run options
    #[clap(default_value_t = 1, long)]
    repeat: usize,
    #[clap(default_value_t = false, long)]
    auto_exit: bool,
    extra_args: Vec<String>,
}

#[unsafe(no_mangle)]
fn main() -> i32 {
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

    let mut has_failed = 0;
    if workloads.len() > 0 {
        for _ in 0..args.repeat {
            let ret = harness::sim_run_multiple(&workloads, true, false, args.auto_exit);
            if ret != 0 {
                has_failed = 1;
                if args.auto_exit {
                    return ret;
                }
            }
            coverage::all_cover_display();
        }
    }

    if args.fuzzing {
        let input_case = harness::load_initial_case(&args.corpus_input);
        // harness::sim_run_with_trackers(&input_case);
        // let original_trakcers = state_tracker::trackers().clone();

        let init_case = input_case;
        // let init_case = if args.reduce {
        //     reduce::reduce_fault_case(
        //         &input_case,
        //         &original_trakcers,
        //         args.reset_vector,
        //         args.save_reduce,
        //         &args.corpus_output,
        //     )
        // } else {
        //     input_case
        // };

        if let Err(e) = fuzzer::run_fuzzer(
            args.max_iters,
            args.max_run_timeout,
            args.top_pass,
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
        }) {
            log::error!("{e}");
            has_failed = 1;
        }
    }

    return has_failed;
}
