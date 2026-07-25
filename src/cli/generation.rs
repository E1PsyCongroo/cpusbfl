use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::common::{CommonArgs, SBFLArgs, parse_positive_u64, parse_positive_usize, parse_weight};
use crate::mutator::PSBFLMutationStrategy;

#[derive(Args, Debug)]
pub(crate) struct GenerationArgs {
    #[arg(long, value_name = "PATH", conflicts_with = "resume_corpus")]
    pub(crate) input: Option<PathBuf>,
    #[arg(default_value_t = false, short = 'r', long, alias = "reduce")]
    pub(crate) reduce_insts: bool,
    #[arg(default_value_t = false, long, requires = "reduce_insts")]
    pub(crate) save_reduce: bool,

    #[arg(
        long,
        value_name = "FILE",
        conflicts_with_all = ["input", "reduce_insts", "save_reduce"]
    )]
    pub(crate) resume_corpus: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    pub(crate) save_corpus: Option<PathBuf>,
    #[arg(long, value_parser = parse_positive_u64, requires = "save_corpus")]
    pub(crate) checkpoint_interval: Option<u64>,
    #[arg(long)]
    pub(crate) gen_only: bool,

    #[arg(long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,

    #[arg(default_value_t = 100, long)]
    pub(crate) max_iters: u64,
    #[arg(default_value_t = 10, value_parser = parse_positive_u64, long)]
    pub(crate) max_run_timeout: u64,

    #[command(flatten)]
    pub(crate) common_args: CommonArgs,

    #[command(flatten)]
    pub(crate) sbfl_args: SBFLArgs,
}

#[derive(Subcommand, Debug)]
pub(crate) enum GenerationMode {
    PSBFL {
        #[arg(default_value_t = 20, value_parser = parse_positive_usize, long)]
        mutator_window_size: usize,
        #[arg(default_value_t = PSBFLMutationStrategy::Uniform, value_enum, long)]
        mutator_weight_strategy: PSBFLMutationStrategy,
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    Random {
        #[arg(last = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },

    WitHW {
        #[arg(default_value_t = 50,value_parser = parse_positive_usize,  long)]
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

impl GenerationMode {
    pub(crate) fn extra_args(&self) -> &[String] {
        match self {
            Self::PSBFL { extra_args, .. }
            | Self::Random { extra_args }
            | Self::WitHW { extra_args, .. } => extra_args,
        }
    }
}
