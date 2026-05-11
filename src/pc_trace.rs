use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::harness::*;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct PCTrace {
    pcs: Vec<u64>,
}

impl PCTrace {
    pub fn new() -> Self {
        Self { pcs: Vec::new() }
    }

    pub fn update_from_pcs(&mut self, pcs: &[u64]) {
        self.pcs = pcs.to_vec();
    }

    pub fn len(&self) -> usize {
        self.pcs.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &u64> {
        self.pcs.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = u64> {
        self.pcs.into_iter()
    }

    pub fn as_slice(&self) -> &[u64] {
        self.pcs.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u64] {
        self.pcs.as_mut_slice()
    }
}

static PC_TRACE: OnceLock<Mutex<PCTrace>> = OnceLock::new();

pub(crate) fn pc_trace_init() {
    let _ = PC_TRACE.set(Mutex::new(PCTrace::new()));
}

pub(crate) fn pc_trace() -> std::sync::MutexGuard<'static, PCTrace> {
    PC_TRACE
        .get()
        .expect("pc_trace_init() not called")
        .lock()
        .expect("poisoned mutex")
}

pub(crate) fn pc_trace_update_stats() {
    let count = unsafe { get_pc_trace_size() };
    let mut pcs = vec![0; count];
    unsafe { update_stats_pc_trace(pcs.as_mut_ptr()) };
    pc_trace().update_from_pcs(&pcs);
}
