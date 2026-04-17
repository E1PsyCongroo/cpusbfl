use std::{borrow::Cow, collections::HashMap, fmt::Debug};

use libafl::{
    Error, HasMetadata, HasNamedMetadata,
    corpus::Testcase,
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
    prelude::NewHashFeedback,
};
use libafl_bolts::{
    Named,
    tuples::{Handle, Handled, MatchName, MatchNameRef},
};
use serde::{Deserialize, Serialize};

use crate::{
    coverage::{cover_accumulate, cover_len},
    observer::multi_coverage_observer::MultiCoverageObserver,
};

pub const COVERAGEFEEDBACK_PREFIX: &str = "multicoveragefeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiCoverageMetadata {
    pub coverage: HashMap<String, Vec<u8>>,
    pub is_passed: bool,
    pub is_initial: bool,
}

libafl_bolts::impl_serdeany!(MultiCoverageMetadata);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiCoverageFeedback<'a> {
    name: Cow<'static, str>,
    o_ref: Handle<MultiCoverageObserver<'a>>,
    inner: NewHashFeedback<MultiCoverageObserver<'a>>,
    pending: Option<MultiCoverageMetadata>,
}

impl<'a> MultiCoverageFeedback<'a> {
    #[must_use]
    pub fn new(observer: &MultiCoverageObserver<'a>) -> Self {
        Self {
            name: Cow::from(COVERAGEFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl Named for MultiCoverageFeedback<'_> {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for MultiCoverageFeedback<'_>
where
    S: HasNamedMetadata,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for MultiCoverageFeedback<'_>
where
    OT: MatchName,
    S: HasNamedMetadata,
{
    fn is_interesting(
        &mut self,
        state: &mut S,
        manager: &mut EM,
        input: &I,
        observers: &OT,
        exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        self.pending = None;

        let interesting = self
            .inner
            .is_interesting(state, manager, input, observers, exit_kind)?;

        if !interesting {
            return Ok(false);
        }

        let obs = observers
            .get(&self.o_ref)
            .expect("A MultiCoverageFeedback needs a BacktraceObserver");

        self.pending = Some(MultiCoverageMetadata {
            coverage: obs.get_coverage_map(),
            is_passed: matches!(exit_kind, ExitKind::Ok),
            is_initial: false,
        });

        Ok(true)
    }

    fn append_metadata(
        &mut self,
        state: &mut S,
        manager: &mut EM,
        observers: &OT,
        testcase: &mut Testcase<I>,
    ) -> Result<(), Error> {
        self.inner
            .append_metadata(state, manager, observers, testcase)?;

        let pending = self
            .pending
            .take()
            .ok_or_else(|| Error::unknown("append_metadata called without pending metadata"))?;

        for (cover_name, cover_points) in pending.coverage.iter() {
            println!(
                "[Debug] COVERAGE: {}, {}, {}",
                cover_name,
                cover_len(&cover_name),
                cover_points.iter().map(|&x| x as u64).sum::<u64>(),
            );
        }

        testcase.add_metadata(pending);
        Ok(())
    }
}
