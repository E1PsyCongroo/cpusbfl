use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};

use libafl::{
    executors::ExitKind,
    observers::Observer,
    prelude::{Error, ObserverWithHashField},
};
use libafl_bolts::{Named, prelude::OwnedPtr};
use serde::{Deserialize, Serialize};

use crate::state_tracker::StateTracker;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StateTrackerObserver {
    name: Cow<'static, str>,
    tracker: OwnedPtr<StateTracker>,
    hash: Option<u64>,
}

impl StateTrackerObserver {
    pub unsafe fn from_raw(name: &'static str, tracker: &StateTracker) -> Self {
        Self {
            name: Cow::Borrowed(name),
            tracker: unsafe { OwnedPtr::from_raw(tracker) },
            hash: None,
        }
    }

    pub fn get_state_tracker(&self) -> &StateTracker {
        self.tracker.as_ref()
    }
}

impl Named for StateTrackerObserver {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, S> Observer<I, S> for StateTrackerObserver {
    fn pre_exec(&mut self, _state: &mut S, _input: &I) -> Result<(), Error> {
        self.hash = None;
        Ok(())
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _input: &I,
        _exit_kind: &ExitKind,
    ) -> Result<(), Error> {
        let mut h = DefaultHasher::new();
        self.get_state_tracker().hash(&mut h);
        self.hash = Some(h.finish());
        Ok(())
    }
}

impl ObserverWithHashField for StateTrackerObserver {
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
