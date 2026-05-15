use std::{borrow::Cow, fmt::Debug, future::Pending};

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

use crate::{coverage::Coverages, observer::coverages_observer::CoveragesObserver};

pub const COVERAGSEFEEDBACK_PREFIX: &str = "coveragesfeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoveragesMetadata {
    pub covers: Coverages,
    pub is_passed: bool,
}

libafl_bolts::impl_serdeany!(CoveragesMetadata);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CoveragesFeedback {
    name: Cow<'static, str>,
    o_ref: Handle<CoveragesObserver>,
    inner: NewHashFeedback<CoveragesObserver>,
    pending: Option<CoveragesMetadata>,
}

impl CoveragesFeedback {
    #[must_use]
    pub fn new(observer: &CoveragesObserver) -> Self {
        Self {
            name: Cow::from(COVERAGSEFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl Named for CoveragesFeedback {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for CoveragesFeedback
where
    S: HasNamedMetadata,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for CoveragesFeedback
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

        let obs = observers
            .get(&self.o_ref)
            .expect("A CoveragesFeedback needs a BacktraceObserver");

        self.pending = Some(CoveragesMetadata {
            covers: obs.get_coverages().to_owned(),
            is_passed: matches!(exit_kind, ExitKind::Ok),
        });

        Ok(interesting)
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

        let pending = self.pending.take().ok_or_else(|| {
            Error::unknown("CoveragesFeedback append_metadata called without pending metadata")
        })?;

        // for cover_name in pending.covers.names() {
        //     println!("[SBFL DEBUG] cover points of {cover_name}:");
        //     for (point, count) in pending
        //         .covers
        //         .get(&cover_name)
        //         .covered_counts()
        //         .into_iter()
        //         .enumerate()
        //     {
        //         println!("[{point}]: {count}");
        //     }
        // }

        testcase.add_metadata(pending);
        Ok(())
    }
}
