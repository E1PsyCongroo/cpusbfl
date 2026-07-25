use std::path::{Path, PathBuf};

use crate::{
    app::{AppResult, ensure_output_dir, write_elapsed},
    bugloc::report_result,
    checkpoint::{self, CheckpointConfig},
    cli::{CommonArgs, SBFLArgs, split_extra_args},
    fuzzer::FuzzSession,
    harness,
    selection::{Selection, emit_top_passed_testcases},
    spectrum::SpectrumMetric,
    utils::process_cpu_time_now,
};

pub(super) fn run(
    coverage_names: String,
    state_names: String,
    input: PathBuf,
    output: Option<PathBuf>,
    common_args: CommonArgs,
    sbfl_args: SBFLArgs,
    extra_args: Vec<String>,
) -> AppResult {
    let (_, emu_args) = split_extra_args(extra_args);
    let checkpoint_config = CheckpointConfig {
        coverage: coverage_names.clone(),
        state: state_names.clone(),
        tracker_window_size: common_args.tracker_window_size,
    };
    let (saved_config, session) = checkpoint::load(&input)?;
    checkpoint::validate_config(&checkpoint_config, &saved_config)?;
    harness::set_sim_env(
        coverage_names,
        state_names,
        emu_args,
        common_args.save_intermediate,
    );

    ensure_output_dir(output.as_ref())?;

    let analysis_start_time = process_cpu_time_now()?;
    emit_and_report(
        &session,
        output.as_ref(),
        common_args.cover_distance_weight,
        common_args.save_intermediate,
        sbfl_args.top_pass,
        sbfl_args.selection,
        sbfl_args.selection_diversity_weight,
        sbfl_args.selection_pool_factor,
        sbfl_args.reduce_cover,
        sbfl_args.top_sus,
        sbfl_args.metric,
        sbfl_args.rtl.rtl_path,
        sbfl_args.rtl.include_paths.as_deref(),
        sbfl_args.rtl.top_module.as_deref(),
        sbfl_args.rtl.top_scope.as_deref(),
    )?;
    let analysis_elapsed = process_cpu_time_now()?
        .checked_sub(analysis_start_time)
        .unwrap_or_default();
    log::info!("SBFL analysis process CPU time = {analysis_elapsed:?}");
    write_elapsed(output, "sbfl_time.txt", analysis_elapsed)
}

pub(super) fn emit_and_report(
    session: &FuzzSession,
    output: Option<impl AsRef<Path>>,
    cover_distance_weight: f64,
    save_intermediate: bool,
    top_pass: usize,
    selection: Selection,
    selection_diversity_weight: f64,
    selection_pool_factor: usize,
    reduce_cover: bool,
    top_sus: u64,
    metric: SpectrumMetric,
    rtl_path: Option<impl AsRef<Path>>,
    include_paths: Option<&[PathBuf]>,
    top_module: Option<&str>,
    top_scope: Option<&str>,
) -> AppResult {
    let passed_cases = emit_top_passed_testcases(
        session,
        output.as_ref(),
        cover_distance_weight,
        save_intermediate,
        top_pass,
        selection,
        selection_diversity_weight,
        selection_pool_factor,
        reduce_cover,
    )?;

    report_result(
        top_sus,
        metric,
        passed_cases,
        rtl_path,
        include_paths,
        top_module,
        top_scope,
        output,
    )
}
