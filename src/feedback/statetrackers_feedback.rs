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

use crate::{observer::statetrackers_observer::StateTrackersObserver, state_tracker::*};

pub const STATETRACKERSFEEDBACK_PREFIX: &str = "statetrackersfeedback_metadata_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTrackersMetadata {
    pub trackers: StateTrackers,
}

libafl_bolts::impl_serdeany!(StateTrackersMetadata);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StateTrackersFeedback {
    name: Cow<'static, str>,
    o_ref: Handle<StateTrackersObserver>,
    inner: NewHashFeedback<StateTrackersObserver>,
    pending: Option<StateTrackersMetadata>,
}

impl StateTrackersFeedback {
    #[must_use]
    pub fn new(observer: &StateTrackersObserver) -> Self {
        Self {
            name: Cow::from(STATETRACKERSFEEDBACK_PREFIX.to_string() + observer.name()),
            o_ref: observer.handle(),
            inner: NewHashFeedback::new(observer),
            pending: None,
        }
    }
}

impl Named for StateTrackersFeedback {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<S> StateInitializer<S> for StateTrackersFeedback
where
    S: HasNamedMetadata,
{
    fn init_state(&mut self, state: &mut S) -> Result<(), Error> {
        self.inner.init_state(state)
    }
}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for StateTrackersFeedback
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

        let trackers = observers
            .get(&self.o_ref)
            .expect("A StateTrackersFeedback needs a BacktraceObserver")
            .get_state_tracker();

        let interesting = self
            .inner
            .is_interesting(state, manager, input, observers, exit_kind)?
            && trackers.len() > 0;

        self.pending = Some(StateTrackersMetadata {
            trackers: trackers.to_owned(),
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

        // println!("[Debug] Appending state trackers metadata:");
        // println!(
        //     "[Debug] State trackers has {} states: {:?}",
        //     self.pending.as_ref().unwrap().trackers.len(),
        //     self.pending.as_ref().unwrap().trackers
        // );

        let pending = self.pending.take().ok_or_else(|| {
            Error::unknown("StateTrackersFeedback append_metadata called without pending metadata")
        })?;

        testcase.add_metadata(pending);
        Ok(())
    }
}
