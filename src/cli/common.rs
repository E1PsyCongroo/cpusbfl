use std::path::PathBuf;

use clap::Args;

use crate::{selection::Selection, spectrum::SpectrumMetric};

pub(super) fn parse_weight(s: &str) -> Result<f64, String> {
    let value = s.parse::<f64>().map_err(|e| e.to_string())?;

    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{value} is not in range [0, 1]"))
    }
}

pub(super) fn parse_positive_usize(s: &str) -> Result<usize, String> {
    let value = s.parse::<usize>().map_err(|e| e.to_string())?;
    if value > 0 {
        Ok(value)
    } else {
        Err(format!("{value} is not a positive integer"))
    }
}

pub(super) fn parse_positive_u64(s: &str) -> Result<u64, String> {
    let value = s.parse::<u64>().map_err(|e| e.to_string())?;
    if value > 0 {
        Ok(value)
    } else {
        Err(format!("{value} is not a positive integer"))
    }
}

#[derive(Args, Debug)]
pub(crate) struct CommonArgs {
    #[arg(default_value_t = 20, value_parser = parse_positive_usize, long)]
    pub(crate) tracker_window_size: usize,
    #[arg(default_value_t = 0.5f64, value_parser = parse_weight, long)]
    pub(crate) cover_distance_weight: f64,

    #[arg(default_value_t = false, long)]
    pub(crate) save_intermediate: bool,
}

#[derive(Args, Debug)]
pub(crate) struct RTLArgs {
    #[arg(
        long,
        value_name = "PATH",
        requires_all = ["top_module", "top_scope"]
    )]
    pub(crate) rtl_path: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATHS",
        value_delimiter = ',',
        requires = "rtl_path"
    )]
    pub(crate) include_paths: Option<Vec<PathBuf>>,
    #[arg(long, requires = "rtl_path")]
    pub(crate) top_module: Option<String>,
    #[arg(long, requires = "rtl_path")]
    pub(crate) top_scope: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct SBFLArgs {
    #[arg(default_value_t = 10, long)]
    pub(crate) top_pass: usize,

    #[arg(default_value_t = Selection::Sort, long, value_enum)]
    pub(crate) selection: Selection,
    #[arg(default_value_t = 0.4f64, value_parser = parse_weight, long)]
    pub(crate) selection_diversity_weight: f64,
    #[arg(default_value_t = 3usize, value_parser = parse_positive_usize, long)]
    pub(crate) selection_pool_factor: usize,

    #[arg(default_value_t = false, long)]
    pub(crate) reduce_cover: bool,

    #[arg(default_value_t = 10, long)]
    pub(crate) top_sus: u64,

    #[command(flatten)]
    pub(crate) rtl: RTLArgs,

    #[arg(default_value_t = SpectrumMetric::Ochiai, long, value_enum)]
    pub(crate) metric: SpectrumMetric,
}
