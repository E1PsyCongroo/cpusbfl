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

use crate::{
    coverage::{Coverage, CoveragePoint},
    observer::coverage_observer::CoverageObserver,
};

pub const COVERAGEFEEDBACK_PREFIX: &str = "coveragefeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "T: CoveragePoint", deserialize = "T: CoveragePoint"))]
pub struct CoverageMetadata<T>
where
    T: CoveragePoint,
{
    pub cover: Coverage<T>,
    pub is_passed: bool,
}

libafl_bolts::impl_serdeany!(CoverageMetadata<T: CoveragePoint>);

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(bound(serialize = "T: CoveragePoint", deserialize = "T: CoveragePoint"))]
pub struct CoverageFeedback<T>
where
    T: CoveragePoint,
{
    name: Cow<'static, str>,
    o_ref: Handle<CoverageObserver<T>>,
    inner: NewHashFeedback<CoverageObserver<T>>,
    pending: Option<CoverageMetadata<T>>,
}

impl<T> CoverageFeedback<T>
where
    T: CoveragePoint,
{
    #[must_use]
    pub fn new(observer: &CoverageObserver<T>) -> Self {
        Self {
            name: Cow::from(COVERAGEFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl<T> Named for CoverageFeedback<T>
where
    T: CoveragePoint,
{
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<T, S> StateInitializer<S> for CoverageFeedback<T>
where
    S: HasNamedMetadata,
    T: CoveragePoint,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<T, EM, I, OT, S> Feedback<EM, I, OT, S> for CoverageFeedback<T>
where
    OT: MatchName,
    S: HasNamedMetadata,
    T: CoveragePoint,
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
            cover: obs.get_coverage().to_owned(),
            is_passed: matches!(exit_kind, ExitKind::Ok),
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
