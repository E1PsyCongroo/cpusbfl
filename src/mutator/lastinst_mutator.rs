use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::{Named, rands::Rand};

const RISCV_BASE: u64 = 0x8000_0000;
const END_INST: [u8; 4] = 0x0000_006b_u32.to_le_bytes();

#[derive(Debug)]
pub(crate) struct LastInstMutator {
    offset: usize,
}

impl LastInstMutator {
    pub(crate) fn new(last_pc: u64) -> Result<Self, Error> {
        let offset = last_pc.checked_sub(RISCV_BASE).ok_or_else(|| {
            Error::illegal_argument(format!("Last PC {last_pc:#x} < BASE {RISCV_BASE:#x}"))
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
        // let halfword = u16::from_le_bytes([mutated_word[0], mutated_word[1]]);
        // let inst_len = if halfword & 0b11 == 0b11 { 4 } else { 2 };
        // let Some(end_start) = self.offset.checked_add(inst_len) else {
        //     return Ok(MutationResult::Skipped);
        // };
        // let Some(end_end) = end_start.checked_add(END_INST.len()) else {
        //     return Ok(MutationResult::Skipped);
        // };

        // if end_end > bytes_len {
        //     return Ok(MutationResult::Skipped);
        // }

        let bytes = input.mutator_bytes_mut();
        bytes[self.offset..mutated_end].copy_from_slice(&mutated_word[..4]);
        // bytes[end_start..end_end].copy_from_slice(&END_INST);
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
