use alloc::borrow::Cow;
use libafl::executors::ExitKind;
use libafl::observers::{MapObserver, Observer, StdMapObserver};
use libafl::prelude::*;
use libafl_bolts::Named;
use serde::{Deserialize, Serialize};

fn stable_hash(bytes: &Vec<u8>) -> u64 {
    use core::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CoverageObserver<'a> {
    name: Cow<'static, str>,
    inner: StdMapObserver<'a, u8, false>,
    hash: Option<u64>,
}

impl<'a> CoverageObserver<'a> {
    pub unsafe fn from_mut_ptr(name: &'static str, map_ptr: *mut u8, map_len: usize) -> Self {
        unsafe {
            Self {
                name: Cow::Borrowed(name),
                inner: StdMapObserver::from_mut_ptr(name, map_ptr, map_len),
                hash: None,
            }
        }
    }

    pub fn coverage_vec(&self) -> Vec<u8> {
        self.inner.to_vec()
    }
}

impl<'a> Named for CoverageObserver<'a> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<'a, I, S> Observer<I, S> for CoverageObserver<'a> {
    fn pre_exec(&mut self, _state: &mut S, _input: &I) -> Result<(), Error> {
        self.inner.reset_map()
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _input: &I,
        _exit_kind: &ExitKind,
    ) -> Result<(), Error> {
        self.hash = Some(stable_hash(&self.inner.to_vec()));
        Ok(())
    }
}

impl<'a> ObserverWithHashField for CoverageObserver<'a> {
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
