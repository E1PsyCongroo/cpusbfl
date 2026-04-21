use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};

use libafl::{
    executors::ExitKind,
    observers::Observer,
    prelude::{Error, ObserverWithHashField},
};
use libafl_bolts::{Named, prelude::OwnedPtr};
use serde::{Deserialize, Serialize};

use crate::coverage::Coverages;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CoveragesObserver {
    name: Cow<'static, str>,
    covers: OwnedPtr<Coverages>,
    hash: Option<u64>,
}

impl CoveragesObserver {
    pub unsafe fn from_raw(name: &'static str, covers: &Coverages) -> Self {
        Self {
            name: Cow::Borrowed(name),
            covers: unsafe { OwnedPtr::from_raw(covers) },
            hash: None,
        }
    }

    pub fn get_coverages(&self) -> &Coverages {
        self.covers.as_ref()
    }
}

impl Named for CoveragesObserver {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, S> Observer<I, S> for CoveragesObserver {
    fn post_exec(
        &mut self,
        _state: &mut S,
        _input: &I,
        _exit_kind: &ExitKind,
    ) -> Result<(), Error> {
        let mut h = DefaultHasher::new();
        self.get_coverages().hash(&mut h);
        self.hash = Some(h.finish());
        Ok(())
    }
}

impl ObserverWithHashField for CoveragesObserver {
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
