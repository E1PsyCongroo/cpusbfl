use std::{borrow::Cow, fmt::Debug};

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

use crate::observer::coverage_observer::CoverageObserver;

pub const COVERAGEFEEDBACK_PREFIX: &str = "coveragefeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageMetadata {
    pub coverage: Vec<u8>,
    pub is_passed: bool,
    pub is_initial: bool,
}

libafl_bolts::impl_serdeany!(CoverageMetadata);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CoverageFeedback<'a> {
    name: Cow<'static, str>,
    o_ref: Handle<CoverageObserver<'a>>,
    inner: NewHashFeedback<CoverageObserver<'a>>,
    pending: Option<CoverageMetadata>,
}

impl<'a> CoverageFeedback<'a> {
    #[must_use]
    pub fn new(observer: &CoverageObserver<'a>) -> Self {
        Self {
            name: Cow::from(COVERAGEFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl Named for CoverageFeedback<'_> {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for CoverageFeedback<'_>
where
    S: HasNamedMetadata,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for CoverageFeedback<'_>
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
            .expect("A CoverageFeedback needs a BacktraceObserver");

        self.pending = Some(CoverageMetadata {
            coverage: obs.coverage_vec(),
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

        testcase.add_metadata(pending);
        Ok(())
    }
}
