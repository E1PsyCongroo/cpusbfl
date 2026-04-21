use std::{
    hash::{Hash, Hasher},
    collections::HashMap,
    ffi::{CStr, CString},
    panic,
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};

use crate::harness::*;

fn set_cover_feedback_by_name(cover_name: &str) {
    unsafe { set_cover_feedback(CString::new(cover_name.as_bytes()).unwrap().as_ptr()) }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct Coverage {
    cover_points: Vec<u8>,
}

impl Coverage {
    pub fn new(n_cover: usize) -> Self {
        Self {
            cover_points: vec![0; n_cover],
        }
    }

    pub fn len(&self) -> usize {
        self.cover_points.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &u8> {
        self.cover_points.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = u8> {
        self.cover_points.into_iter()
    }

    pub fn as_slice(&self) -> &[u8] {
        self.cover_points.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.cover_points.as_mut_slice()
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.cover_points.as_ptr()
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.cover_points.as_ptr().cast_mut()
    }
}

impl Hash for Coverage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.cover_points.hash(state);
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct Coverages {
    covers: HashMap<String, Coverage>,
}

impl Coverages {
    pub fn new(cover_names: &[String]) -> Self {
        let mut covers = HashMap::new();

        for cover_name in cover_names {
            set_cover_feedback_by_name(cover_name);
            let n_cover = unsafe { get_cover_number() as usize };
            covers.insert(cover_name.clone(), Coverage::new(n_cover));
        }

        Self { covers }
    }

    pub fn len(&self) -> usize {
        self.covers.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Coverage)> {
        self.covers.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (String, Coverage)> {
        self.covers.into_iter()
    }

    pub fn get(&self, cover_name: &str) -> &Coverage {
        self.covers
            .get(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }

    pub fn get_mut(&mut self, cover_name: &str) -> &mut Coverage {
        self.covers
            .get_mut(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }

    pub fn names(&self) -> Vec<String> {
        self.covers.keys().cloned().collect()
    }
}

impl Hash for Coverages {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut entries: Vec<_> = self.covers.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        for (name, coverage) in entries {
            name.hash(state);
            coverage.hash(state);
        }
    }
}

struct AccumulatedCoverages {
    covers: HashMap<String, Vec<u8>>,
}

impl AccumulatedCoverages {
    fn new(cover_names: &[String]) -> Self {
        let mut covers = HashMap::new();

        for cover_name in cover_names {
            set_cover_feedback_by_name(cover_name);
            let n_cover = unsafe { get_cover_number() as usize };
            covers.insert(cover_name.clone(), vec![0; n_cover]);
        }

        Self { covers }
    }

    fn get(&self, cover_name: &str) -> &Vec<u8> {
        self.covers
            .get(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }

    fn get_mut(&mut self, cover_name: &str) -> &mut Vec<u8> {
        self.covers
            .get_mut(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }
}

static COVERAGES: OnceLock<Mutex<Coverages>> = OnceLock::new();
static ACCUMULATED_COVERAGES: OnceLock<Mutex<AccumulatedCoverages>> = OnceLock::new();

/// Call this once, right after your C test‑bench has told you how many
/// counters are present.
pub(crate) fn cover_init(cover_names: Vec<String>) {
    let _ = COVERAGES.set(Mutex::new(Coverages::new(&cover_names)));
    let _ = ACCUMULATED_COVERAGES.set(Mutex::new(AccumulatedCoverages::new(&cover_names)));
}

pub(crate) fn coverages() -> std::sync::MutexGuard<'static, Coverages> {
    COVERAGES
        .get()
        .expect("cover_init() not called")
        .lock()
        .expect("poisoned mutex")
}

fn accumulated_coverages() -> std::sync::MutexGuard<'static, AccumulatedCoverages> {
    ACCUMULATED_COVERAGES
        .get()
        .expect("cover_init() not called")
        .lock()
        .expect("poisoned mutex")
}

fn get_accumulative_coverage(cover_name: &str) -> f64 {
    let guard = accumulated_coverages();
    let accumulated_cov = guard.get(cover_name);
    let mut covered_num: usize = 0;
    for covered in accumulated_cov.iter() {
        if *covered != 0 as u8 {
            covered_num += 1;
        }
    }
    100.0 * covered_num as f64 / accumulated_cov.len() as f64
}

pub(crate) fn cover_names() -> Vec<String> {
    coverages().names()
}

pub(crate) fn cover_len(cover_name: &str) -> usize {
    coverages().get(cover_name).len()
}

pub(crate) fn cover_point_name(cover_name: &str, i: usize) -> String {
    let cover_point_name = unsafe {
        set_cover_feedback_by_name(cover_name);
        get_cover_point_name(i)
    };
    if cover_point_name.is_null() {
        format!("{}[{}]", cover_name, i)
    } else {
        unsafe { CStr::from_ptr(cover_point_name) }
            .to_str()
            .map(|s| s.to_owned())
            .unwrap_or_else(|_| format!("{}[{}]", cover_name, i))
    }
}

pub(crate) fn cover_as_mut_ptr(cover_name: &str) -> *mut u8 {
    coverages().get_mut(cover_name).as_mut_ptr().cast::<u8>()
}

pub(crate) fn cover_update_stats(cover_name: &str) {
    unsafe {
        set_cover_feedback_by_name(cover_name);
        update_stats_cover(cover_as_mut_ptr(cover_name));
    }
}

pub(crate) fn cover_accumulate(cover_name: &str) {
    let cov_guard = coverages();
    let cov = cov_guard.get(cover_name);
    let mut accumulated_cov_guard = accumulated_coverages();
    let accumulated_cov = accumulated_cov_guard.get_mut(cover_name);
    for (i, covered) in cov.cover_points.iter().enumerate() {
        if *covered != 0 as u8 {
            accumulated_cov[i] = 1;
        }
    }
}

pub(crate) fn cover_display(cover_name: &str) {
    println!(
        "{} Accumulative Coverage:       {:.3}%",
        cover_name,
        get_accumulative_coverage(cover_name)
    )
}

pub(crate) fn all_cover_update_stats() {
    for cover_name in cover_names() {
        cover_update_stats(&cover_name);
    }
}

pub(crate) fn all_cover_accumulate() {
    for cover_name in cover_names() {
        cover_accumulate(&cover_name);
    }
}

pub(crate) fn all_cover_display() {
    for cover_name in cover_names() {
        cover_display(&cover_name);
    }
}
