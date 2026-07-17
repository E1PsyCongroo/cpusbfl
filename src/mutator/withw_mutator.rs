use std::borrow::Cow;
use std::collections::HashSet;

use libafl::prelude::*;
use libafl_bolts::{Named, rands::Rand};

use crate::elf::*;
use crate::feedback::*;
use crate::inst::*;
use crate::mutator::MutationMetadata;
use crate::similarity::*;
use crate::state_tracker::*;

const MIN_PRIORITY: f64 = 1.0e-6;

#[derive(Debug, Clone)]
struct WitHWCandidate {
    pc: u64,
    offset: usize,
    len: usize,
    priority: f64,
}

#[derive(Debug)]
pub(crate) struct WitHWMutator {
    candidates: Vec<WitHWCandidate>,
    cover_weight: f64,
    mutation_count: usize,
    priority_alpha: f64,
    failed_reward: f64,

    mutated_pcs: HashSet<u64>,
}

impl WitHWMutator {
    pub(crate) fn new<I>(
        init_bytes: &I,
        pc_trace: &StateTracker<PCState>,
        window_size: u64,
        cover_weight: f64,
        mutate_rate: f64,
        priority_alpha: f64,
        failed_reward: f64,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        I: HasMutatorBytes,
    {
        if window_size == 0 {
            return Err("window_size must be greater than 0".into());
        }
        if !(0.0..=1.0).contains(&mutate_rate) {
            return Err("mutate_rate must be in range [0, 1]".into());
        }
        if !(0.0..=1.0).contains(&priority_alpha) {
            return Err("priority_alpha must be in range [0, 1]".into());
        }
        if !failed_reward.is_finite() || failed_reward < 0.0 {
            return Err("failed_reward must be a finite non-negative number".into());
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
        let mut seen = HashSet::new();
        let window = trace[start..]
            .iter()
            .copied()
            .filter(|pc| seen.insert(*pc))
            .collect::<Vec<_>>();

        let mut candidates = Vec::new();
        for pc in window {
            let offset = usize::try_from(elf_parser.vma2offset(pc)?)?;
            let inst_len = inst_len_at(init_bytes, offset);

            candidates.push(WitHWCandidate {
                pc,
                offset,
                len: inst_len,
                priority: 1.0,
            });
        }

        if candidates.is_empty() {
            return Err(format!(
                "no executable mutation candidates found in the last {window_size} PCs"
            )
            .into());
        }

        let mutation_count = (((candidates.len() as f64) * mutate_rate).floor() as usize).max(1);

        Ok(Self {
            candidates,
            mutation_count,
            priority_alpha,
            failed_reward,
            cover_weight,
            mutated_pcs: HashSet::new(),
        })
    }

    fn pick_candidate_indices<R: Rand>(&self, rand: &mut R) -> Vec<usize> {
        let mut available = (0..self.candidates.len()).collect::<Vec<_>>();
        let mut selected = Vec::with_capacity(self.mutation_count);

        while !available.is_empty() && selected.len() < self.mutation_count {
            let total_priority = available
                .iter()
                .map(|idx| self.candidates[*idx].priority.max(MIN_PRIORITY))
                .sum::<f64>();
            let mut ticket = rand.next_float() * total_priority;
            let mut selected_pos = available.len() - 1;

            for (pos, idx) in available.iter().copied().enumerate() {
                let priority = self.candidates[idx].priority.max(MIN_PRIORITY);
                if ticket < priority {
                    selected_pos = pos;
                    break;
                }
                ticket -= priority;
            }

            selected.push(available.swap_remove(selected_pos));
        }

        selected
    }

    fn fill_random_bytes<R: Rand>(rand: &mut R, dst: &mut [u8]) {
        for chunk in dst.chunks_mut(8) {
            let random = rand.next().to_le_bytes();
            chunk.copy_from_slice(&random[..chunk.len()]);
        }
    }

    fn accepted_case_diversity<I, S>(&self, state: &S, id: CorpusId) -> Result<f64, Error>
    where
        S: HasCorpus<I>,
    {
        let testcase = state.corpus().get(id)?.borrow();
        let covers = &testcase.metadata::<CoveragesMetadata>()?.covers;
        let trackers = &testcase.metadata::<StateTrackersMetadata>()?.trackers;

        let mut distances = Vec::new();
        for other_id in state.corpus().ids() {
            if other_id == id {
                continue;
            }

            let other = state.corpus().get(other_id)?.borrow();
            if !other.metadata::<PassedMetadata>()?.is_passed {
                continue;
            }

            let other_covers = &other.metadata::<CoveragesMetadata>()?.covers;
            let other_trackers = &other.metadata::<StateTrackersMetadata>()?.trackers;
            let cover_distance = coverage_distance(covers, other_covers);
            let state_distance = state_trackers_distance(trackers, other_trackers);
            distances.push(combine_raw_distance(
                cover_distance,
                state_distance,
                self.cover_weight,
            ));
        }

        if distances.is_empty() {
            Ok(1.0)
        } else {
            Ok(distances.iter().sum::<f64>() / distances.len() as f64)
        }
    }

    fn update_priorities(&mut self, reward: f64) {
        let reward = if reward.is_finite() {
            reward.max(MIN_PRIORITY)
        } else {
            MIN_PRIORITY
        };

        for candidate in &mut self.candidates {
            if self.mutated_pcs.contains(&candidate.pc) {
                candidate.priority = ((1.0 - self.priority_alpha) * candidate.priority
                    + self.priority_alpha * reward)
                    .max(MIN_PRIORITY);
            }
        }
    }
}

impl<I, S> Mutator<I, S> for WitHWMutator
where
    S: HasCorpus<I> + HasMetadata + HasRand + HasTestcase<I>,
    I: HasMutatorBytes,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        self.mutated_pcs.clear();

        let selected = self.pick_candidate_indices(state.rand_mut());
        if selected.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        for candidate_idx in selected {
            let candidate = &self.candidates[candidate_idx];
            let Some(mutated_end) = candidate.offset.checked_add(candidate.len) else {
                continue;
            };
            if mutated_end > input.mutator_bytes().len() {
                continue;
            }

            let dst = &mut input.mutator_bytes_mut()[candidate.offset..mutated_end];
            Self::fill_random_bytes(state.rand_mut(), dst);
            self.mutated_pcs.insert(candidate.pc);
        }

        if self.mutated_pcs.is_empty() {
            Ok(MutationResult::Skipped)
        } else {
            Ok(MutationResult::Mutated)
        }
    }

    fn post_exec(&mut self, state: &mut S, new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        if let Some(id) = new_corpus_id {
            let (parent_id, is_passed) = {
                let testcase = state.testcase(id)?;
                (
                    testcase.parent_id(),
                    testcase.metadata::<PassedMetadata>()?.is_passed,
                )
            };

            if !self.mutated_pcs.is_empty() {
                let parent_mutated_pcs = match parent_id {
                    Some(parent_id) => state
                        .testcase(parent_id)?
                        .metadata::<MutationMetadata>()
                        .map(|metadata| metadata.mutated_pcs.clone())
                        .unwrap_or_default(),
                    None => HashSet::new(),
                };
                let mut mutated_pcs = parent_mutated_pcs;
                mutated_pcs.extend(self.mutated_pcs.iter().copied());
                state.testcase_mut(id)?.add_metadata(MutationMetadata { mutated_pcs });
            }

            let reward = if is_passed {
                self.accepted_case_diversity(state, id)?
            } else {
                self.failed_reward
            };
            self.update_priorities(reward);
        }

        Ok(())
    }
}

impl Named for WitHWMutator {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("WitHWMutator");
        &NAME
    }
}
