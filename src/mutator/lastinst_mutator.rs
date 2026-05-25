use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::{Named, rands::Rand};

#[derive(Debug)]
pub(crate) struct LastInstMutator {
    offset: usize,
}

impl LastInstMutator {
    pub(crate) fn new(reset_vector: u64, last_pc: u64) -> Result<Self, Error> {
        let offset = last_pc.checked_sub(reset_vector).ok_or_else(|| {
            Error::illegal_argument(format!("Last PC {last_pc:#x} < BASE {reset_vector:#x}"))
        })?;
        let offset = usize::try_from(offset).map_err(|_| {
            Error::illegal_argument(format!("Last PC offset {offset:#x} does not fit in usize"))
        })?;

        Ok(Self { offset })
    }
}

impl<I, S> Mutator<I, S> for LastInstMutator
where
    S: HasRand,
    I: HasMutatorBytes,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        let Some(mutated_end) = self.offset.checked_add(4) else {
            return Ok(MutationResult::Skipped);
        };

        let bytes_len = input.mutator_bytes().len();
        if mutated_end > bytes_len {
            return Ok(MutationResult::Skipped);
        }

        let mutated_word = state.rand_mut().next().to_le_bytes();
        let bytes = input.mutator_bytes_mut();
        bytes[self.offset..mutated_end].copy_from_slice(&mutated_word[..4]);
        Ok(MutationResult::Mutated)
    }

    #[inline]
    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl Named for LastInstMutator {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("LastInstMutator");
        &NAME
    }
}
