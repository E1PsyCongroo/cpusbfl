use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};

use libafl::{
    executors::ExitKind,
    observers::Observer,
    prelude::{Error, ObserverWithHashField},
};
use libafl_bolts::{Named, prelude::OwnedPtr};
use serde::{Deserialize, Serialize};

use crate::coverage::{Coverage, CoveragePoint};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(bound(serialize = "T: CoveragePoint", deserialize = "T: CoveragePoint"))]
pub struct CoverageObserver<T>
where
    T: CoveragePoint,
{
    name: Cow<'static, str>,
    cover: OwnedPtr<Coverage<T>>,
    hash: Option<u64>,
}

impl<T> CoverageObserver<T>
where
    T: CoveragePoint,
{
    pub unsafe fn from_raw(name: &'static str, cover: &Coverage<T>) -> Self {
        Self {
            name: Cow::Borrowed(name),
            cover: unsafe { OwnedPtr::from_raw(cover) },
            hash: None,
        }
    }

    pub fn get_coverage(&self) -> &Coverage<T> {
        self.cover.as_ref()
    }
}

impl<T> Named for CoverageObserver<T>
where
    T: CoveragePoint,
{
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<T, I, S> Observer<I, S> for CoverageObserver<T>
where
    T: CoveragePoint,
{
    fn post_exec(
        &mut self,
        _state: &mut S,
        _input: &I,
        _exit_kind: &ExitKind,
    ) -> Result<(), Error> {
        let mut h = DefaultHasher::new();
        self.get_coverage().hash(&mut h);
        self.hash = Some(h.finish());
        Ok(())
    }
}

impl<T> ObserverWithHashField for CoverageObserver<T>
where
    T: CoveragePoint,
{
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
