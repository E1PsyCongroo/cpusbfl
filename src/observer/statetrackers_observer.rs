use std::{
    arch,
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
pub(crate) struct StateTrackersObserver {
    name: Cow<'static, str>,
    trackers: OwnedPtr<StateTrackers>,
    window_size: usize,
    hash: Option<u64>,
}

impl StateTrackersObserver {
    pub(crate) unsafe fn from_raw(
        name: &'static str,
        trackers: &StateTrackers,
        window_size: usize,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            trackers: unsafe { OwnedPtr::from_raw(trackers) },
            window_size: window_size,
            hash: None,
        }
    }

    pub(crate) fn get_state_tracker(&self) -> StateTrackers {
        let trackers = self.trackers.as_ref();

        let mut pc_tracker = trackers.pc_tracker.clone();
        let mut arch_int_reg_tracker = trackers.arch_int_reg_tracker.clone();
        let mut csr_tracker = trackers.csr_tracker.clone();

        let drop_len = self
            .trackers
            .as_ref()
            .len()
            .saturating_sub(self.window_size);
        drop(pc_tracker.drain(..drop_len));
        drop(arch_int_reg_tracker.drain(..drop_len));
        drop(csr_tracker.drain(..drop_len));

        StateTrackers {
            state_names: trackers.state_names.clone(),
            pc_tracker,
            arch_int_reg_tracker,
            csr_tracker,
        }
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
