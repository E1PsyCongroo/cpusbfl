use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::{Named, rands::Rand};

use crate::elf::*;
use crate::inst::*;
use crate::state_tracker::*;

#[derive(Debug, Clone)]
struct LastWindowCandidate {
    offset: usize,
    len: usize,
    weight: u64,
}

#[derive(Debug)]
pub(crate) struct LastWindowMutator {
    candidates: Vec<LastWindowCandidate>,
    total_weight: u64,
}

impl LastWindowMutator {
    pub(crate) fn new<I>(
        init_bytes: &I,
        pc_trace: &StateTracker<PCState>,
        window_size: u64,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        I: HasMutatorBytes,
    {
        if window_size == 0 {
            return Err("window_size must be greater than 0".into());
        }

        let init_bytes = init_bytes.mutator_bytes();
        let elf_parser = ELFParser::from_bytes(init_bytes)?;

        let trace = first_dynamic_entries(pc_trace)
            .into_iter()
            .map(|(_, pc)| pc)
            .collect::<Vec<_>>();
        if trace.is_empty() {
            return Err("pc_trace is empty".into());
        }

        let window_size = usize::try_from(window_size)?;
        let start = trace.len().saturating_sub(window_size);
        let window = &trace[start..];

        let mut candidates = Vec::new();
        let mut total_weight = 0u64;

        for (idx, &pc) in window.iter().enumerate() {
            let offset = usize::try_from(elf_parser.vma2offset(pc)?)?;

            let inst_len = inst_len_at(init_bytes, offset);

            let rank = u64::try_from(idx + 1)?;
            let weight = rank
                .checked_mul(rank)
                .ok_or("LastWindowMutator weight overflow")?;

            total_weight = total_weight
                + weight
                    .checked_add(weight)
                    .ok_or("LastWindowMutator total_weight overflow")?;

            candidates.push(LastWindowCandidate {
                offset,
                len: inst_len,
                weight,
            });
        }

        if candidates.is_empty() {
            return Err(format!(
                "no executable mutation candidates found in the last {window_size} PCs"
            )
            .into());
        }

        Ok(Self {
            candidates,
            total_weight,
        })
    }

    fn pick_candidate_idx<R: Rand>(&self, rand: &mut R) -> Option<usize> {
        if self.candidates.is_empty() || self.total_weight == 0 {
            return None;
        }

        let mut ticket = rand.next() % self.total_weight;

        for (idx, candidate) in self.candidates.iter().enumerate() {
            if ticket < candidate.weight {
                return Some(idx);
            }

            ticket -= candidate.weight;
        }

        Some(self.candidates.len() - 1)
    }

    fn fill_random_bytes<R: Rand>(rand: &mut R, dst: &mut [u8]) {
        for chunk in dst.chunks_mut(4) {
            let random = rand.next().to_le_bytes();
            chunk.copy_from_slice(&random[..chunk.len()]);
        }
    }
}

impl<I, S> Mutator<I, S> for LastWindowMutator
where
    S: HasRand,
    I: HasMutatorBytes,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        let Some(candidate_idx) = self.pick_candidate_idx(state.rand_mut()) else {
            return Ok(MutationResult::Skipped);
        };

        let candidate = &self.candidates[candidate_idx];

        let Some(mutated_end) = candidate.offset.checked_add(candidate.len) else {
            return Ok(MutationResult::Skipped);
        };

        assert!(mutated_end <= input.mutator_bytes().len());

        let bytes = input.mutator_bytes_mut();
        let dst = &mut bytes[candidate.offset..mutated_end];

        Self::fill_random_bytes(state.rand_mut(), dst);

        Ok(MutationResult::Mutated)
    }

    #[inline]
    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl Named for LastWindowMutator {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("LastWindowMutator");
        &NAME
    }
}
