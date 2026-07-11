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
#[serde(bound(serialize = "T: State", deserialize = "T: State",))]
pub(crate) struct StateTrackerObserver<T>
where
    T: State,
{
    name: Cow<'static, str>,
    tracker: OwnedPtr<StateTracker<T>>,
    hash: Option<u64>,
}

impl<T> StateTrackerObserver<T>
where
    T: State,
{
    pub(crate) unsafe fn from_raw(name: &'static str, tracker: &StateTracker<T>) -> Self {
        Self {
            name: Cow::Borrowed(name),
            tracker: unsafe { OwnedPtr::from_raw(tracker) },
            hash: None,
        }
    }

    pub(crate) fn get_state_tracker(&self) -> &StateTracker<T> {
        self.tracker.as_ref()
    }
}

impl<T> Named for StateTrackerObserver<T>
where
    T: State,
{
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<T, I, S> Observer<I, S> for StateTrackerObserver<T>
where
    T: State,
{
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

impl<T> ObserverWithHashField for StateTrackerObserver<T>
where
    T: State,
{
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
