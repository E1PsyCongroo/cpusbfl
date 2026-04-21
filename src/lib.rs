mod coverage;
mod feedback;
mod fuzzer;
mod harness;
mod monitor;
mod observer;
mod state_tracker;
mod bugloc;
mod similarity;

use clap::Parser;

#[derive(Parser, Default, Debug)]
struct Arguments {
    // Fuzzer options
    #[clap(default_value_t = false, short, long)]
    fuzzing: bool,
    #[clap(default_value_t = String::from("llvm.branch"), short, long)]
    coverage: String,
    #[clap(default_value_t = false, short, long)]
    verbose: bool,
    #[clap(default_value_t = 100, long)]
    max_iters: u64,
    #[clap(default_value_t = 10, long)]
    max_run_timeout: u64,
    #[clap(default_value_t = 10, long)]
    top_pass: u64,
    #[clap(default_value_t = 10, long)]
    top_sus: u64,
    #[clap(default_value_t = String::from("./corpus"), long)]
    corpus_input: String,
    #[clap(long)]
    corpus_output: Option<String>,
    #[clap(default_value_t = false, long)]
    save_errors: bool,
    // Run options
    #[clap(default_value_t = 1, long)]
    repeat: usize,
    #[clap(default_value_t = false, long)]
    auto_exit: bool,
    extra_args: Vec<String>,
}

#[unsafe(no_mangle)]
fn main() -> i32 {
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

    harness::set_sim_env(args.coverage, args.verbose, emu_args);

    let mut has_failed = 0;
    if workloads.len() > 0 {
        for _ in 0..args.repeat {
            let ret = harness::sim_run_multiple(&workloads, args.auto_exit);
            if ret != 0 {
                has_failed = 1;
                if args.auto_exit {
                    return ret;
                }
            }
        }
        coverage::all_cover_display();
    }

    if args.fuzzing {
        if let Ok(passed_cov) = fuzzer::run_fuzzer(
            args.max_iters,
            args.max_run_timeout,
            args.top_pass,
            args.corpus_input,
            args.corpus_output,
        ) {
            bugloc::report_suspicious(&passed_cov, args.top_sus as usize);
        } else {
            has_failed = 1;
        }
    }

    return has_failed;
}
