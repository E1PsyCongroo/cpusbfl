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

use crate::{observer::statetracker_observer::StateTrackerObserver, state_tracker::*};

pub const STATETRACKERFEEDBACK_PREFIX: &str = "statetrackerfeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "T: State", deserialize = "T: State",))]
pub struct StateTrackerMetadata<T>
where
    T: State,
{
    pub tracker: StateTracker<T>,
}

libafl_bolts::impl_serdeany!(
    StateTrackerMetadata<T: State>,
    <PCState>,
    <ArchIntRegState>,
    <CSRState>
);

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(bound(serialize = "T: State", deserialize = "T: State",))]
pub struct StateTrackerFeedback<T>
where
    T: State,
{
    name: Cow<'static, str>,
    o_ref: Handle<StateTrackerObserver<T>>,
    inner: NewHashFeedback<StateTrackerObserver<T>>,
    pending: Option<StateTrackerMetadata<T>>,
}

impl<T> StateTrackerFeedback<T>
where
    T: State,
{
    #[must_use]
    pub fn new(observer: &StateTrackerObserver<T>) -> Self {
        Self {
            name: Cow::from(STATETRACKERFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl<T> Named for StateTrackerFeedback<T>
where
    T: State,
{
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<T, S> StateInitializer<S> for StateTrackerFeedback<T>
where
    T: State,
    S: HasNamedMetadata,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<T, EM, I, OT, S> Feedback<EM, I, OT, S> for StateTrackerFeedback<T>
where
    T: State,
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

        let tracker = observers
            .get(&self.o_ref)
            .expect("A StateTrackerFeedback needs a BacktraceObserver")
            .get_state_tracker();

        let interesting = self
            .inner
            .is_interesting(state, manager, input, observers, exit_kind)?
            && tracker.len() > 0;

        self.pending = Some(StateTrackerMetadata {
            tracker: tracker.to_owned(),
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

        log::debug!("Appending state tracker metadata:");
        log::debug!(
            "State tracker has {} states: {:?}",
            self.pending.as_ref().unwrap().tracker.len(),
            self.pending.as_ref().unwrap().tracker
        );

        let pending = self.pending.take().ok_or_else(|| {
            Error::unknown("StateTrackerFeedback append_metadata called without pending metadata")
        })?;

        testcase.add_metadata(pending);
        Ok(())
    }
}
