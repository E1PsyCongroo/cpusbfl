use libafl::{inputs::BytesInput, schedulers::QueueScheduler};

use crate::{
    app::AppResult,
    cli::{GenerationArgs, GenerationMode},
    fuzzer::{self, FuzzSession},
    mutator::{BoundedMutator, ELFHavocScheduledMutator, PSBFLMutator, WitHWMutator},
    scheduler::WitScheduler,
};

pub(super) fn run<CF>(
    mode: GenerationMode,
    init_case: Option<&BytesInput>,
    resume: Option<FuzzSession>,
    gen_args: &GenerationArgs,
    checkpoint_callback: CF,
) -> AppResult<FuzzSession>
where
    CF: FnMut(&FuzzSession) -> AppResult,
{
    match mode {
        GenerationMode::PSBFL {
            mutator_weight_strategy,
            mutator_window_size,
            ..
        } => {
            let scheduler = QueueScheduler::new();
            fuzzer::run_fuzzer(
                init_case,
                resume,
                gen_args.output.as_ref(),
                gen_args.max_iters,
                gen_args.max_run_timeout,
                gen_args.common_args.tracker_window_size,
                gen_args.common_args.save_intermediate,
                gen_args.checkpoint_interval,
                || Ok(scheduler),
                |init_input, metadata, _| {
                    PSBFLMutator::new(
                        init_input,
                        &metadata.state_trackers.pc_tracker,
                        mutator_weight_strategy,
                        mutator_window_size,
                    )
                },
                checkpoint_callback,
            )
        }
        GenerationMode::Random { .. } => {
            let scheduler = QueueScheduler::new();
            fuzzer::run_fuzzer(
                init_case,
                resume,
                gen_args.output.as_ref(),
                gen_args.max_iters,
                gen_args.max_run_timeout,
                gen_args.common_args.tracker_window_size,
                gen_args.common_args.save_intermediate,
                gen_args.checkpoint_interval,
                || Ok(scheduler),
                |init_input, _, _| {
                    ELFHavocScheduledMutator::new(libafl::mutators::havoc_mutations(), init_input)
                },
                checkpoint_callback,
            )
        }
        GenerationMode::WitHW {
            max_corpus_size,
            init_seed_rate,
            mutate_rate,
            priority_alpha,
            failed_reward,
            ..
        } => {
            let initial_corpus_id = resume.as_ref().map(|session| session.initial_corpus_id);
            let scheduler = WitScheduler::new(
                init_seed_rate,
                gen_args.common_args.cover_distance_weight,
                initial_corpus_id,
            );
            fuzzer::run_fuzzer(
                init_case,
                resume,
                gen_args.output.as_ref(),
                gen_args.max_iters,
                gen_args.max_run_timeout,
                gen_args.common_args.tracker_window_size,
                gen_args.common_args.save_intermediate,
                gen_args.checkpoint_interval,
                || Ok(scheduler),
                |init_input, metadata, state| {
                    let mut mutator = WitHWMutator::new(
                        init_input,
                        &metadata.state_trackers.pc_tracker,
                        gen_args.common_args.tracker_window_size,
                        gen_args.common_args.cover_distance_weight,
                        mutate_rate,
                        priority_alpha,
                        failed_reward,
                    )?;
                    mutator.restore_priorities(state);
                    Ok(BoundedMutator::new(
                        mutator,
                        max_corpus_size,
                        gen_args.common_args.cover_distance_weight,
                    ))
                },
                checkpoint_callback,
            )
        }
    }
}
