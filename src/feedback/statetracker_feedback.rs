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

use crate::{observer::statetracker_observer::StateTrackerObserver, state_tracker::StateTracker};

pub const STATETRACKERFEEDBACK_PREFIX: &str = "statetrackerfeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTrackerMetadata {
    pub track: StateTracker,
    pub is_passed: bool,
}

libafl_bolts::impl_serdeany!(StateTrackerMetadata);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StateTrackerFeedback {
    name: Cow<'static, str>,
    o_ref: Handle<StateTrackerObserver>,
    inner: NewHashFeedback<StateTrackerObserver>,
    pending: Option<StateTrackerMetadata>,
}

impl StateTrackerFeedback {
    #[must_use]
    pub fn new(observer: &StateTrackerObserver) -> Self {
        Self {
            name: Cow::from(STATETRACKERFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl Named for StateTrackerFeedback {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for StateTrackerFeedback
where
    S: HasNamedMetadata,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for StateTrackerFeedback
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

        let track = observers
            .get(&self.o_ref)
            .expect("A StateTrackerFeedback needs a BacktraceObserver")
            .get_state_tracker();

        let interesting = self
            .inner
            .is_interesting(state, manager, input, observers, exit_kind)?
            && track.len() > 0;

        self.pending = Some(StateTrackerMetadata {
            track: track.to_owned(),
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

        // println!("[Debug] Appending state tracker metadata:");
        // println!(
        //     "[Debug] State tracker has {} states of size {}",
        //     self.pending.as_ref().unwrap().track.len(),
        //     self.pending.as_ref().unwrap().track.state_size()
        // );

        let pending = self
            .pending
            .take()
            .ok_or_else(|| Error::unknown("StateTrackerFeedback append_metadata called without pending metadata"))?;

        testcase.add_metadata(pending);
        Ok(())
    }
}
