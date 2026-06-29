use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::{Named, rands::Rand};

use crate::elf::*;
use crate::inst::*;

#[derive(Debug)]
pub(crate) struct LastInstMutator {
    offset: usize,
    len: usize,
}

impl LastInstMutator {
    pub(crate) fn new(elf_bytes: &[u8], last_pc: u64) -> Result<Self, Box<dyn std::error::Error>> {
        let elf_parser = ELFParser::from_bytes(elf_bytes)?;
        let min_inst_end = last_pc + COMPRESSED_INST_BYTES as u64;
        let offset = usize::try_from(elf_parser.vma2offset(last_pc, min_inst_end).ok_or_else(
            || {
                format!(
                    "failed to convert last_pc VMA range to file offset: \
                         vma={last_pc:#x}..{min_inst_end:#x}"
                )
            },
        )?)?;

        let inst_len = inst_len_at(elf_bytes, offset);

        Ok(Self {
            offset: offset,
            len: inst_len,
        })
    }
}

impl<I, S> Mutator<I, S> for LastInstMutator
where
    S: HasRand,
    I: HasMutatorBytes,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        let Some(mutated_end) = self.offset.checked_add(self.len) else {
            return Ok(MutationResult::Skipped);
        };

        let bytes_len = input.mutator_bytes().len();
        if mutated_end > bytes_len {
            return Ok(MutationResult::Skipped);
        }

        let mutated_word = state.rand_mut().next().to_le_bytes();
        let bytes = input.mutator_bytes_mut();
        bytes[self.offset..mutated_end].copy_from_slice(&mutated_word[..self.len]);
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
