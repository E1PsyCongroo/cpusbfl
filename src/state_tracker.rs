use std::{
    cmp::max,
    ffi::{CString, c_void},
    fmt::Debug,
    hash::{Hash, Hasher},
    panic,
    sync::{Mutex, OnceLock},
};

use dtw_rs::{Distance, Midpoint};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::harness::*;

pub(crate) trait State:
    Copy
    + Clone
    + Default
    + Debug
    + Hash
    + Eq
    + PartialEq
    + Serialize
    + DeserializeOwned
    + 'static
    + Distance<Output = f64>
    + Midpoint
{
}

#[derive(Copy, Clone, Default, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PCState {
    pub value: u64,
}

impl State for PCState {}

impl Hash for PCState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl Distance for PCState {
    type Output = f64;

    fn distance(&self, other: &Self) -> Self::Output {
        if self.value == other.value { 0.0 } else { 1.0 }
    }
}

impl Midpoint for PCState {
    fn midpoint(&self, _other: &Self) -> Self {
        self.clone()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ArchIntRegState {
    pub value: [u64; 32],
}

impl State for ArchIntRegState {}

impl Hash for ArchIntRegState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl Distance for ArchIntRegState {
    type Output = f64;

    fn distance(&self, other: &Self) -> Self::Output {
        self.value
            .iter()
            .zip(other.value.iter())
            .filter(|(a, b)| a != b)
            .count() as f64
            / self.value.len() as f64
    }
}

impl Midpoint for ArchIntRegState {
    fn midpoint(&self, _other: &Self) -> Self {
        self.clone()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CSRState {
    pub privilege_mode: u64,
    pub mstatus: u64,
    pub sstatus: u64,
    pub mepc: u64,
    pub sepc: u64,
    pub mtval: u64,
    pub stval: u64,
    pub mtvec: u64,
    pub stvec: u64,
    pub mcause: u64,
    pub scause: u64,
    pub satp: u64,
    pub mip: u64,
    pub mie: u64,
    pub mscratch: u64,
    pub sscratch: u64,
    pub mideleg: u64,
    pub medeleg: u64,
}

impl State for CSRState {}

impl Hash for CSRState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.privilege_mode.hash(state);
        self.mstatus.hash(state);
        self.sstatus.hash(state);
        self.mepc.hash(state);
        self.sepc.hash(state);
        self.mtval.hash(state);
        self.stval.hash(state);
        self.mtvec.hash(state);
        self.stvec.hash(state);
        self.mcause.hash(state);
        self.scause.hash(state);
        self.satp.hash(state);
        self.mip.hash(state);
        self.mie.hash(state);
        self.mscratch.hash(state);
        self.sscratch.hash(state);
        self.mideleg.hash(state);
        self.medeleg.hash(state);
    }
}

impl Distance for CSRState {
    type Output = f64;

    fn distance(&self, other: &Self) -> Self::Output {
        let mut diff = 0u32;
        diff += (self.privilege_mode != other.privilege_mode) as u32;
        diff += (self.mstatus != other.mstatus) as u32;
        diff += (self.mepc != other.mepc) as u32;
        diff += (self.sepc != other.sepc) as u32;
        diff += (self.mtval != other.mtval) as u32;
        diff += (self.stval != other.stval) as u32;
        diff += (self.mtvec != other.mtvec) as u32;
        diff += (self.stvec != other.stvec) as u32;
        diff += (self.mcause != other.mcause) as u32;
        diff += (self.scause != other.scause) as u32;
        diff += (self.satp != other.satp) as u32;
        diff += (self.mip != other.mip) as u32;
        diff += (self.mie != other.mie) as u32;
        diff += (self.mscratch != other.mscratch) as u32;
        diff += (self.sscratch != other.sscratch) as u32;
        diff += (self.mideleg != other.mideleg) as u32;
        diff += (self.medeleg != other.medeleg) as u32;
        (diff as f64) / 17.0
    }
}

impl Midpoint for CSRState {
    fn midpoint(&self, _other: &Self) -> Self {
        self.clone()
    }
}

#[derive(Clone, Default, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(bound(serialize = "T: State", deserialize = "T: State",))]
pub struct StateTracker<T>
where
    T: State,
{
    name: String,
    tracker: Vec<T>,
}

impl<T> StateTracker<T>
where
    T: State,
{
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            tracker: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.tracker.len()
    }

    pub fn update(&mut self) {
        unsafe {
            set_state_feedback(CString::new(self.name.as_str()).unwrap().as_ptr());
            self.tracker.clear();
            self.tracker.resize_with(get_state_number(), T::default);
            update_stats_state(self.tracker.as_mut_ptr() as *mut c_void);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.tracker.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = T> {
        self.tracker.into_iter()
    }

    pub fn as_slice(&self) -> &[T] {
        self.tracker.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.tracker.as_mut_slice()
    }
}

impl<T> Hash for StateTracker<T>
where
    T: State,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tracker.hash(state);
    }
}

#[derive(Clone, Default, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct StateTrackers {
    pub(crate) state_names: Vec<String>,
    pub(crate) pc_tracker: StateTracker<PCState>,
    pub(crate) arch_int_reg_tracker: StateTracker<ArchIntRegState>,
    pub(crate) csr_tracker: StateTracker<CSRState>,
}

impl StateTrackers {
    pub fn new(state_names: Vec<String>) -> Self {
        Self {
            state_names: state_names,
            pc_tracker: StateTracker::new("PCState"),
            arch_int_reg_tracker: StateTracker::new("ArchIntRegState"),
            csr_tracker: StateTracker::new("CSRState"),
        }
    }

    pub fn len(&self) -> usize {
        let pc_tracker_len = self.pc_tracker.len();
        let arch_int_reg_tracker_len = self.arch_int_reg_tracker.len();
        let csr_tracker_len = self.csr_tracker.len();
        let len = max(
            pc_tracker_len,
            max(arch_int_reg_tracker_len, csr_tracker_len),
        );

        if self.state_names.contains(&self.pc_tracker.name) {
            assert_eq!(len, pc_tracker_len);
        }
        if self.state_names.contains(&self.arch_int_reg_tracker.name) {
            assert_eq!(len, arch_int_reg_tracker_len);
        }
        if self.state_names.contains(&self.csr_tracker.name) {
            assert_eq!(len, csr_tracker_len);
        }

        len
    }

    pub fn update(&mut self) {
        for state_name in &self.state_names {
            match state_name.as_str() {
                "PCState" => self.pc_tracker.update(),
                "ArchIntRegState" => self.arch_int_reg_tracker.update(),
                "CSRState" => self.csr_tracker.update(),
                _ => panic!("unknown state name: {}", state_name),
            }
        }
    }
}

impl Hash for StateTrackers {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.state_names.contains(&self.pc_tracker.name) {
            self.pc_tracker.hash(state);
        }
        if self.state_names.contains(&self.arch_int_reg_tracker.name) {
            self.arch_int_reg_tracker.hash(state);
        }
        if self.state_names.contains(&self.csr_tracker.name) {
            self.csr_tracker.hash(state);
        }
    }
}

static STATE_TRACKERS: OnceLock<Mutex<StateTrackers>> = OnceLock::new();

pub(crate) fn state_tracker_init(state_names: Vec<String>) {
    let _ = STATE_TRACKERS.set(Mutex::new(StateTrackers::new(state_names)));
}

pub(crate) fn trackers() -> std::sync::MutexGuard<'static, StateTrackers> {
    STATE_TRACKERS
        .get()
        .expect("state_tracker_init() not called")
        .lock()
        .expect("poisoned mutex")
}

pub(crate) fn state_names() -> Vec<String> {
    trackers().state_names.clone()
}

pub(crate) fn all_tracker_update() {
    trackers().update();
}
