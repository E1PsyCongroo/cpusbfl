use std::borrow::Cow;

use libafl::prelude::*;
use libafl_bolts::Named;

pub(crate) enum SelectedMutator<I, S> {
    First(Box<dyn Mutator<I, S>>),
    Second(Box<dyn Mutator<I, S>>),
}

impl<I, S> Named for SelectedMutator<I, S> {
    fn name(&self) -> &Cow<'static, str> {
        match self {
            SelectedMutator::First(m) => m.name(),
            SelectedMutator::Second(m) => m.name(),
        }
    }
}

impl<I, S> Mutator<I, S> for SelectedMutator<I, S>
where
    I: HasMutatorBytes,
    S: HasCorpus<I> + HasMetadata + HasRand + HasTestcase<I>,
{
    #[inline]
    fn mutate(&mut self, state: &mut S, input: &mut I) -> Result<MutationResult, Error> {
        match self {
            SelectedMutator::First(m) => m.mutate(state, input),
            SelectedMutator::Second(m) => m.mutate(state, input),
        }
    }

    #[inline]
    fn post_exec(&mut self, state: &mut S, new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        match self {
            SelectedMutator::First(m) => m.post_exec(state, new_corpus_id),
            SelectedMutator::Second(m) => m.post_exec(state, new_corpus_id),
        }
    }
}
