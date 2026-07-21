use libafl::inputs::BytesInput;

use crate::{
    app::{AppResult, write_elapsed},
    cli::GenerationArgs,
    harness, reduce, state_tracker, utils,
};

pub(super) fn prepare_new_initial_case(gen_args: &GenerationArgs) -> AppResult<Option<BytesInput>> {
    if gen_args.resume_corpus.is_some() {
        return Ok(None);
    }

    let corpus_input = gen_args
        .input
        .as_deref()
        .unwrap_or(std::path::Path::new("corpus"));
    let input_case = utils::load_initial_case(corpus_input)?;
    if !gen_args.reduce_insts {
        return Ok(Some(input_case));
    }

    harness::fuzz_harness(&input_case);
    let original_trackers = state_tracker::trackers().clone();
    let reducing_start_time = utils::process_cpu_time_now()?;
    let reduced_case = reduce::reduce_fault_case(
        input_case,
        original_trackers,
        gen_args.save_reduce,
        gen_args.output.as_ref(),
    );
    let reducing_elapsed = utils::process_cpu_time_now()?
        .checked_sub(reducing_start_time)
        .unwrap_or_default();
    log::info!("Reducing process CPU time = {reducing_elapsed:?}");
    write_elapsed(
        gen_args.output.as_deref(),
        "reducing_time.txt",
        reducing_elapsed,
    )?;

    Ok(Some(reduced_case))
}
