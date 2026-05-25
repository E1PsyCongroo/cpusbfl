use std::{
    borrow::Cow,
    fmt::Debug,
    hash::{DefaultHasher, Hash, Hasher},
};

use libafl::{
    executors::ExitKind,
    observers::Observer,
    prelude::{Error, ObserverWithHashField},
};
use libafl_bolts::{Named, prelude::OwnedPtr};
use serde::{Deserialize, Serialize};

use crate::state_tracker::*;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StateTrackersObserver {
    name: Cow<'static, str>,
    trackers: OwnedPtr<StateTrackers>,
    hash: Option<u64>,
}

impl StateTrackersObserver {
    pub unsafe fn from_raw(name: &'static str, trackers: &StateTrackers) -> Self {
        Self {
            name: Cow::Borrowed(name),
            trackers: unsafe { OwnedPtr::from_raw(trackers) },
            hash: None,
        }
    }

    pub fn get_state_tracker(&self) -> &StateTrackers {
        self.trackers.as_ref()
    }
}

impl Named for StateTrackersObserver {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, S> Observer<I, S> for StateTrackersObserver {
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

impl ObserverWithHashField for StateTrackersObserver {
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
