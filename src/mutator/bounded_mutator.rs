use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::Named;

use crate::similarity::*;

pub(crate) struct BoundedMutator<M> {
    inner: M,
    max_corpus_size: usize,
    cover_weight: f64,
    name: Cow<'static, str>,
}

impl<M> BoundedMutator<M>
where
    M: Named,
{
    pub(crate) fn new(inner: M, max_corpus_size: usize, cover_weight: f64) -> Self {
        Self {
            name: Cow::Owned(format!("Bounded[{}]", inner.name())),
            inner,
            max_corpus_size,
            cover_weight,
        }
    }
}

impl<M> Named for BoundedMutator<M> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, M, S> Mutator<I, S> for BoundedMutator<M>
where
    I: HasMutatorBytes,
    M: Mutator<I, S>,
    S: HasCorpus<I> + HasMetadata + HasRand + HasTestcase<I>,
{
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        self.inner.mutate(state, input)
    }

    fn post_exec(&mut self, state: &mut S, new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        self.inner.post_exec(state, new_corpus_id)?;

        if new_corpus_id.is_none() {
            return Ok(());
        }

        let initial_id = state
            .corpus()
            .first()
            .ok_or_else(|| Error::empty("corpus has no initial testcase".to_string()))?;
        let mut ranked = ranked_passed_cases(state, initial_id, self.cover_weight)?;

        if ranked.len() > self.max_corpus_size {
            ranked.sort_by(|a, b| {
                a.distance
                    .total_cmp(&b.distance)
                    .then_with(|| a.id.cmp(&b.id))
            });
            let victims = ranked.split_off(self.max_corpus_size);
            for victim in victims {
                if *state.corpus().current() == Some(victim.id) {
                    *state.corpus_mut().current_mut() = None;
                }
                state.corpus_mut().remove(victim.id)?;
                log::debug!(
                    "Evicted corpus testcase {} at distance {:.6}; pass corpus limit={}",
                    victim.id,
                    victim.distance,
                    self.max_corpus_size,
                );
            }
        }

        Ok(())
    }
}
