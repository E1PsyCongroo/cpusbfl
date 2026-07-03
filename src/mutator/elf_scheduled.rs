//! The `ScheduledMutator` schedules multiple mutations internally.

use core::fmt::Debug;
use std::borrow::Cow;

use libafl::{
    Error,
    corpus::CorpusId,
    inputs::{BytesInput, HasMutatorBytes},
    mutators::{HavocScheduledMutator, MutationResult, Mutator, MutatorsTuple},
    state::HasRand,
};

use libafl_bolts::{HasLen, Named, Truncate, tuples::NamedTuple};
use lief::generic::Section;

use crate::elf::*;

/// A [`Mutator`] that stacks embedded mutations in a havoc manner on each call.
#[derive(Debug)]
pub struct ELFHavocScheduledMutator<MT> {
    name: Cow<'static, str>,
    mutator: HavocScheduledMutator<MT>,
    text_section_max_size: usize,
}

impl<MT> Named for ELFHavocScheduledMutator<MT> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, MT, S> Mutator<I, S> for ELFHavocScheduledMutator<MT>
where
    I: HasMutatorBytes,
    MT: MutatorsTuple<BytesInput, S>,
    S: HasRand,
{
    #[inline]
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        let input_bytes = input.mutator_bytes_mut();
        let mut elf_parser = ELFParser::from_bytes(input_bytes)
            .map_err(|e| Error::illegal_argument(e.to_string()))?;
        match elf_parser.with_text_section_mut(|text_section| -> Result<MutationResult, Error> {
            let text_section = text_section
                .as_mut()
                .ok_or(Error::illegal_argument(format!(
                    "failed to find .text section"
                )))?;

            let text_section_size = usize::try_from(text_section.size())?;

            if text_section_size > self.text_section_max_size {
                return Err(Error::illegal_argument(format!(
                    "Text section size {} exceeds maximum allowed size {}",
                    text_section_size, self.text_section_max_size
                )));
            }

            let text_section_start = usize::try_from(text_section.file_offset())?;
            let text_section_end = text_section_start + text_section_size;
            let mut text_input =
                BytesInput::new(input_bytes[text_section_start..text_section_end].to_vec());

            match self.mutator.mutate(state, &mut text_input)? {
                MutationResult::Skipped => Ok(MutationResult::Skipped),
                MutationResult::Mutated => {
                    let mut mutated_text = text_input.mutator_bytes().to_vec();

                    if mutated_text.len() > self.text_section_max_size {
                        mutated_text.truncate(self.text_section_max_size);
                    }

                    let mutated_text_size = mutated_text.len();

                    text_section.set_content(&mutated_text);
                    text_section.set_size(u64::try_from(mutated_text_size)?);

                    Ok(MutationResult::Mutated)
                }
            }
        })? {
            MutationResult::Skipped => Ok(MutationResult::Skipped),
            MutationResult::Mutated => {
                let new_elf_bytes = elf_parser
                    .into_bytes()
                    .map_err(|e| Error::illegal_state(e.to_string()))?;

                if new_elf_bytes.len() > input_bytes.len() {
                    return Err(Error::illegal_state(format!(
                        "Mutated ELF size {} exceeds original size {}",
                        new_elf_bytes.len(),
                        input.len()
                    )));
                }

                let (prefix, suffix) = input_bytes.split_at_mut(new_elf_bytes.len());

                prefix.copy_from_slice(&new_elf_bytes);
                suffix.fill(0);

                Ok(MutationResult::Mutated)
            }
        }
    }

    #[inline]
    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl<MT> ELFHavocScheduledMutator<MT>
where
    MT: NamedTuple,
{
    /// Create a new [`ELFHavocScheduledMutator`] instance specifying mutations
    pub fn new<I>(mutations: MT, init_bytes: &I) -> Result<Self, Box<dyn std::error::Error>>
    where
        I: HasMutatorBytes,
    {
        Ok(ELFHavocScheduledMutator {
            name: Cow::from(format!(
                "ELFHavocScheduledMutator[{}]",
                mutations.names().join(", ")
            )),
            mutator: HavocScheduledMutator::new(mutations),
            text_section_max_size: usize::try_from(
                ELFParser::from_bytes(init_bytes.mutator_bytes())?
                    .borrow_text_section()
                    .as_ref()
                    .ok_or(format!(""))?
                    .size(),
            )?,
        })
    }

    /// Create a new [`ELFHavocScheduledMutator`] instance specifying mutations and the maximun number of iterations
    #[inline]
    pub fn with_max_stack_pow<I>(
        mutations: MT,
        init_bytes: &I,
        max_stack_pow: usize,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        I: HasMutatorBytes,
    {
        Ok(Self {
            name: Cow::from(format!(
                "ELFHavocScheduledMutator[{}]",
                mutations.names().join(", ")
            )),
            mutator: HavocScheduledMutator::with_max_stack_pow(mutations, max_stack_pow),
            text_section_max_size: usize::try_from(
                ELFParser::from_bytes(init_bytes.mutator_bytes())?
                    .borrow_text_section()
                    .as_ref()
                    .ok_or(format!(""))?
                    .size(),
            )?,
        })
    }
}
