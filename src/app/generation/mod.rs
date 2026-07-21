mod input;
mod strategy;

use crate::{
    app::{AppResult, analysis::emit_and_report, ensure_output_dir, write_elapsed},
    checkpoint::{self, CheckpointConfig},
    cli::{GenerationArgs, GenerationMode, split_extra_args},
    fuzzer::FuzzSession,
    harness,
    utils::process_cpu_time_now,
};

pub(super) fn run(
    coverage_names: String,
    state_names: String,
    gen_mode: GenerationMode,
    gen_args: GenerationArgs,
) -> AppResult {
    let (_, emu_args) = split_extra_args(gen_mode.extra_args().to_vec());
    let saved_config = CheckpointConfig {
        coverage: coverage_names.clone(),
        state: state_names.clone(),
        tracker_window_size: gen_args.common_args.tracker_window_size,
    };
    harness::set_sim_env(coverage_names, state_names, emu_args);

    ensure_output_dir(gen_args.output.as_deref())?;

    let generation_start_time = process_cpu_time_now()?;
    let resume = if let Some(path) = gen_args.resume_corpus.as_ref() {
        let (checkpoint_config, session) = checkpoint::load(path)?;
        checkpoint::validate_config(&checkpoint_config, &saved_config)?;
        Some(session)
    } else {
        None
    };
    let init_case = input::prepare_new_initial_case(&gen_args)?;
    let save_corpus = gen_args.save_corpus.clone();
    let checkpoint_callback = |session: &FuzzSession| {
        if let Some(path) = save_corpus.as_ref() {
            checkpoint::save(path, &saved_config, session)?;
        }
        Ok(())
    };

    let session = strategy::run(
        gen_mode,
        init_case.as_ref(),
        resume,
        &gen_args,
        checkpoint_callback,
    )?;

    if let Some(path) = gen_args.save_corpus.as_ref() {
        checkpoint::save(path, &saved_config, &session)?;
    }

    let generation_elapsed = process_cpu_time_now()?
        .checked_sub(generation_start_time)
        .unwrap_or_default();
    log::info!("Generation process CPU time = {generation_elapsed:?}");
    write_elapsed(gen_args.output.as_ref(), "gen_time.txt", generation_elapsed)?;

    if !gen_args.gen_only {
        let analysis_start_time = process_cpu_time_now()?;
        emit_and_report(
            &session,
            gen_args.output.as_ref(),
            gen_args.common_args.cover_distance_weight,
            gen_args.common_args.save_intermediate,
            gen_args.sbfl_args.top_pass,
            gen_args.sbfl_args.selection,
            gen_args.sbfl_args.selection_diversity_weight,
            gen_args.sbfl_args.selection_pool_factor,
            gen_args.sbfl_args.reduce_cover,
            gen_args.sbfl_args.top_sus,
            gen_args.sbfl_args.metric,
            gen_args.sbfl_args.rtl.rtl_path,
            gen_args.sbfl_args.rtl.include_paths.as_deref(),
            gen_args.sbfl_args.rtl.top_module.as_deref(),
            gen_args.sbfl_args.rtl.top_scope.as_deref(),
        )?;
        let analysis_elapsed = process_cpu_time_now()?
            .checked_sub(analysis_start_time)
            .unwrap_or_default();
        log::info!("SBFL analysis process CPU time = {analysis_elapsed:?}");
        write_elapsed(gen_args.output, "sbfl_time.txt", analysis_elapsed)?;
    }

    Ok(())
}
