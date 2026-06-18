use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::{Named, rands::Rand};

use crate::elf::elf_vma2offset;

const MUTATED_INST_BYTES: usize = 4;
#[derive(Debug)]
pub(crate) struct LastInstMutator {
    offset: usize,
}

impl LastInstMutator {
    pub(crate) fn new(elf_bytes: &[u8], last_pc: u64) -> Result<Self, Error> {
        let offset = elf_vma2offset(elf_bytes, last_pc, last_pc + MUTATED_INST_BYTES as u64)?;

        Ok(Self { offset })
    }
}

impl<I, S> Mutator<I, S> for LastInstMutator
where
    S: HasRand,
    I: HasMutatorBytes,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        let Some(mutated_end) = self.offset.checked_add(MUTATED_INST_BYTES) else {
            return Ok(MutationResult::Skipped);
        };

        let bytes_len = input.mutator_bytes().len();
        if mutated_end > bytes_len {
            return Ok(MutationResult::Skipped);
        }

        let mutated_word = state.rand_mut().next().to_le_bytes();
        let bytes = input.mutator_bytes_mut();
        bytes[self.offset..mutated_end].copy_from_slice(&mutated_word[..MUTATED_INST_BYTES]);
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
