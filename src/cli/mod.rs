mod common;
mod generation;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

pub(crate) use common::{CommonArgs, SBFLArgs};
pub(crate) use generation::{GenerationArgs, GenerationMode};

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    Workload {
        #[arg(default_value_t = 1, long)]
        repeat: usize,
        #[arg(default_value_t = false, long)]
        auto_exit: bool,
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
    Generation {
        #[command(subcommand)]
        gen_mode: GenerationMode,
        #[command(flatten)]
        gen_args: GenerationArgs,
    },
    Analysis {
        #[arg(long, value_name = "FILE")]
        input: PathBuf,
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,

        #[command(flatten)]
        common_args: CommonArgs,
        #[command(flatten)]
        sbfl_args: SBFLArgs,

        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
    #[arg(default_value_t = String::from("verilator.branch,verilator.line"), short, long)]
    pub(crate) coverage: String,
    #[arg(default_value_t = String::from("PCState,ArchIntRegState,CSRState"), short, long)]
    pub(crate) state: String,
}

pub(crate) fn split_extra_args(extra_args: Vec<String>) -> (Vec<String>, Vec<String>) {
    let split_index = extra_args
        .iter()
        .position(|arg| arg.starts_with('-'))
        .unwrap_or(extra_args.len());

    let mut workloads = extra_args;
    let emu_args = workloads.split_off(split_index);

    (workloads, emu_args)
}
