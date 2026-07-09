use std::borrow::Cow;

use clap::ValueEnum;
use libafl::{HasMetadata, corpus::Corpus, prelude::*, state::HasCorpus};
use libafl_bolts::{Named, rands::Rand};
use serde::{Deserialize, Serialize};

use crate::elf::*;
use crate::inst::*;
use crate::state_tracker::*;

#[derive(Debug, Clone, ValueEnum)]
pub(crate) enum MutationStrategy {
    #[value(name = "uniform")]
    Uniform,
    #[value(name = "tail_linear")]
    TailLinear,
    #[value(name = "tail_quad")]
    TailQuadratic,
    #[value(name = "head_linear")]
    HeadLinear,
    #[value(name = "head_quad")]
    HeadQuadratic,
}

impl Default for MutationStrategy {
    fn default() -> Self {
        Self::Uniform
    }
}

#[derive(Debug, Clone)]
struct LastWindowCandidate {
    offset: usize,
    len: usize,
    weight: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastWindowMutationMetadata {
    pub candidate_idxs: Vec<usize>,
}

libafl_bolts::impl_serdeany!(LastWindowMutationMetadata);

#[derive(Debug)]
pub(crate) struct LastWindowMutator {
    candidates: Vec<LastWindowCandidate>,
    total_weight: u64,
    iters: u64,
    mutated_candidate_idxs: Vec<usize>,
}

impl LastWindowMutator {
    pub(crate) fn new<I>(
        init_bytes: &I,
        pc_trace: &StateTracker<PCState>,
        strategy: MutationStrategy,
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

            let weight = match strategy {
                MutationStrategy::Uniform => 1u64,
                MutationStrategy::TailLinear => u64::try_from(idx + 1)?,
                MutationStrategy::TailQuadratic => u64::try_from(
                    (idx + 1)
                        .checked_pow(2)
                        .ok_or("LastWindowMutator weight overflow")?,
                )?,
                MutationStrategy::HeadLinear => u64::try_from(window.len() - idx)?,
                MutationStrategy::HeadQuadratic => u64::try_from(
                    (window.len() - idx)
                        .checked_pow(2)
                        .ok_or("LastWindowMutator weight overflow")?,
                )?,
            };

            total_weight = total_weight
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

        let iters = std::cmp::max(1, (candidates.len() as f64).sqrt().floor() as u64);
        Ok(Self {
            candidates,
            total_weight,
            iters,
            mutated_candidate_idxs: Vec::new(),
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

    fn mutate_one<S, I>(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error>
    where
        S: HasRand,
        I: HasMutatorBytes,
    {
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
        self.mutated_candidate_idxs.push(candidate_idx);

        Ok(MutationResult::Mutated)
    }
}

impl<I, S> Mutator<I, S> for LastWindowMutator
where
    S: HasRand + HasCorpus<I>,
    I: HasMutatorBytes,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        self.mutated_candidate_idxs.clear();

        let mut r = MutationResult::Skipped;
        for _ in 0..self.iters {
            let outcome = self.mutate_one(state, input)?;
            if outcome == MutationResult::Mutated {
                r = MutationResult::Mutated;
            }
        }
        Ok(r)
    }

    #[inline]
    fn post_exec(&mut self, state: &mut S, new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        if let Some(id) = new_corpus_id
            && !self.mutated_candidate_idxs.is_empty()
        {
            let mut testcase = state.corpus_mut().get(id)?.borrow_mut();
            testcase.add_metadata(LastWindowMutationMetadata {
                candidate_idxs: self.mutated_candidate_idxs.clone(),
            });
        }

        self.mutated_candidate_idxs.clear();
        Ok(())
    }
}

impl Named for LastWindowMutator {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("LastWindowMutator");
        &NAME
    }
}
