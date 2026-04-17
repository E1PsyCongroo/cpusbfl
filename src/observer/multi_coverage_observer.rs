use std::borrow::Cow;
use std::collections::HashMap;

use libafl::{
    executors::ExitKind,
    observers::Observer,
    prelude::{Error, ObserverWithHashField},
};
use libafl_bolts::{AsSlice, AsSliceMut, Named, ownedref::OwnedMutSlice};
use serde::{Deserialize, Serialize};

fn stable_hash(bytes: &Vec<u8>) -> u64 {
    use core::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MultiCoverageObserver<'a> {
    name: Cow<'static, str>,
    covers: HashMap<String, OwnedMutSlice<'a, u8>>,
    hash: Option<u64>,
}

impl<'a> MultiCoverageObserver<'a> {
    pub unsafe fn from_mut_ptr(
        name: &'static str,
        covers: HashMap<String, (*mut u8, usize)>,
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            covers: covers
                .into_iter()
                .map(|(cover_name, (cover_ptr, cover_len))| unsafe {
                    (
                        cover_name.to_owned(),
                        OwnedMutSlice::from_raw_parts_mut(cover_ptr, cover_len),
                    )
                })
                .collect(),
            hash: None,
        }
    }

    pub fn get_coverage_map(&self) -> HashMap<String, Vec<u8>> {
        self.covers
            .iter()
            .map(|(cover_name, cover_points)| {
                (cover_name.clone(), cover_points.as_slice().to_vec())
            })
            .collect()
    }
}

impl<'a> Named for MultiCoverageObserver<'a> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

impl<I, S> Observer<I, S> for MultiCoverageObserver<'_> {
    fn pre_exec(&mut self, _state: &mut S, _input: &I) -> Result<(), Error> {
        for cover in self.covers.values_mut() {
            cover.as_slice_mut().fill(0);
        }
        Ok(())
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _input: &I,
        _exit_kind: &ExitKind,
    ) -> Result<(), Error> {
        let mut cover_names = self.covers.keys().collect::<Vec<_>>();
        cover_names.sort();

        let total_len: usize = cover_names
            .iter()
            .map(|name| self.covers[*name].len())
            .sum();

        let mut all_bytes = Vec::with_capacity(total_len);

        for cover_name in cover_names {
            all_bytes.extend_from_slice(self.covers[cover_name].as_slice());
        }

        self.hash = Some(stable_hash(&all_bytes));
        Ok(())
    }
}

impl<'a> ObserverWithHashField for MultiCoverageObserver<'a> {
    fn hash(&self) -> Option<u64> {
        self.hash
    }
}
