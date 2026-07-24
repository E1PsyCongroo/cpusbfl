mod analysis;
mod generation;
mod workload;

use std::{io::Write, path::Path};

use clap::Parser;

use crate::cli::{Cli, Command};

type AppResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn ensure_output_dir(output: Option<impl AsRef<Path>>) -> AppResult {
    if let Some(output_dir) = output {
        std::fs::create_dir_all(output_dir)?;
    }
    Ok(())
}

fn write_elapsed(
    output: Option<impl AsRef<Path>>,
    filename: &str,
    elapsed: std::time::Duration,
) -> AppResult {
    if let Some(output_dir) = output {
        let output_dir = output_dir.as_ref().join("elapsed");
        std::fs::create_dir_all(&output_dir)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output_dir.join(filename))?
            .write_all(format!("{elapsed:?}").as_bytes())?;
    }
    Ok(())
}

pub(crate) fn run() -> AppResult {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let Cli {
        command,
        coverage,
        state,
    } = Cli::parse();

    match command {
        Command::Workload {
            repeat,
            auto_exit,
            extra_args,
        } => workload::run(coverage, state, repeat, auto_exit, extra_args),
        Command::Generation { gen_mode, gen_args } => {
            generation::run(coverage, state, gen_mode, gen_args)
        }
        Command::Analysis {
            input,
            output,
            common_args,
            sbfl_args,
            extra_args,
        } => analysis::run(
            coverage,
            state,
            input,
            output,
            common_args,
            sbfl_args,
            extra_args,
        ),
    }
}
