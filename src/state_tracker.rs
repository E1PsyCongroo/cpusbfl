use std::ffi::CString;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use dtw_rs::{Distance, Midpoint};

use crate::similarity::*;
use crate::harness::*;

fn set_state_feedback_by_name(state_name: &str) {
    unsafe { set_state_feedback(CString::new(state_name.as_bytes()).unwrap().as_ptr()) }
}


#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct State {
    bytes: Vec<u8>,
}

impl State {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.bytes.as_mut_slice()
    }
}

impl Hash for State {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Distance for State {
    type Output = f64;

    fn distance(&self, other: &Self) -> Self::Output {
        assert_eq!(self.len(), other.len(), "state size mismatch");

        if self.is_empty() {
            return 0.0;
        }

        euclidean_distance(self.as_slice(), other.as_slice())
    }
}

impl Midpoint for State {
    fn midpoint(&self, other: &Self) -> Self {
        assert_eq!(self.len(), other.len(), "state size mismatch");

        Self::new(
            self.as_slice()
                .iter()
                .zip(other.as_slice().iter())
                .map(|(a, b)| ((*a as u16 + *b as u16) / 2) as u8)
                .collect(),
        )
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct StateTracker {
    track: Vec<State>,
    state_size: usize,
}

impl StateTracker {
    pub fn new(state_size: usize) -> Self {
        Self {
            track: Vec::new(),
            state_size: state_size,
        }
    }

    pub fn len(&self) -> usize {
        self.track.len()
    }

    pub fn state_size(&self) -> usize {
        self.state_size
    }

    pub fn update_from_bytes(&mut self, bytes: &[u8], state_size: usize) {
        self.state_size = state_size;
        self.track.clear();

        if state_size == 0 {
            return;
        }

        self.track.extend(
            bytes
                .chunks_exact(state_size)
                .map(|state| State::new(state.to_vec())),
        );
    }

    pub fn iter(&self) -> impl Iterator<Item = &State> {
        self.track.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = State> {
        self.track.into_iter()
    }

    pub fn as_slice(&self) -> &[State] {
        self.track.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [State] {
        self.track.as_mut_slice()
    }
}

impl Hash for StateTracker {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.track.hash(state);
    }
}

static STATE_TRACKER: OnceLock<Mutex<StateTracker>> = OnceLock::new();

pub(crate) fn state_tracker_init(state_name: String) {
    set_state_feedback_by_name(&state_name);
    let state_size = unsafe { get_state_size() };
    let _ = STATE_TRACKER.set(Mutex::new(StateTracker::new(state_size)));
}

pub(crate) fn tracker(_state_name: &str) -> std::sync::MutexGuard<'static, StateTracker> {
    STATE_TRACKER
        .get()
        .expect("state_tracker_init() not called")
        .lock()
        .expect("poisoned mutex")
}

pub(crate) fn state_names() -> Vec<String> {
    vec!["ArchIntRegState".to_string()]
}

pub(crate) fn tracker_len(state_name: &str) -> usize {
    tracker(state_name).len()
}

pub(crate) fn tracker_state_size(state_name: &str) -> usize {
    tracker(state_name).state_size()
}

pub(crate) fn tracker_update_stats(state_name: &str) {
    let mut guard = tracker(state_name);
    unsafe {
        set_state_feedback_by_name(state_name);
        let state_number = get_state_number();
        let state_size = get_state_size();
        let byte_len = state_number * state_size;
        let mut bytes = vec![0; byte_len];
        if byte_len != 0 {
            update_stats_state(bytes.as_mut_ptr().cast());
        }
        guard.update_from_bytes(&bytes, state_size);
    }
}

pub(crate) fn all_tracker_update_stats() {
    for state_name in state_names() {
        tracker_update_stats(&state_name);
    }
}
