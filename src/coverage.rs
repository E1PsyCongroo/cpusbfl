use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_void},
    fmt::Debug,
    hash::{Hash, Hasher},
    panic,
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::harness::{set_cover_feedback as ffi_set_cover_feedback, *};

unsafe fn set_cover_feedback(cover_name: &str) {
    unsafe { ffi_set_cover_feedback(CString::new(cover_name.as_bytes()).unwrap().as_ptr()) }
}

pub(crate) trait CoveragePoint:
    Copy + Clone + Default + Debug + Hash + Eq + PartialEq + Serialize + DeserializeOwned + 'static
{
    fn is_covered(self) -> bool {
        self != Self::default()
    }

    fn as_u64(self) -> u64;
}

impl CoveragePoint for bool {
    fn as_u64(self) -> u64 {
        self as u64
    }
}

impl CoveragePoint for u8 {
    fn as_u64(self) -> u64 {
        self as u64
    }
}

impl CoveragePoint for u64 {
    fn as_u64(self) -> u64 {
        self
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(bound(serialize = "T: CoveragePoint", deserialize = "T: CoveragePoint"))]
pub(crate) struct Coverage<T = u64>
where
    T: CoveragePoint,
{
    name: String,
    point_counts: Vec<T>,
}

impl<T> Coverage<T>
where
    T: CoveragePoint,
{
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            point_counts: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.point_counts.len()
    }

    pub fn update(&mut self) {
        unsafe {
            set_cover_feedback(&self.name);
            let cover_num = get_cover_number();
            if self.point_counts.is_empty() {
                self.point_counts.resize(cover_num as usize, T::default());
            } else {
                assert_eq!(self.point_counts.len(), cover_num as usize);
            }
            update_stats_cover(self.point_counts.as_mut_ptr() as *mut c_void);
        }
    }

    pub fn as_slice(&self) -> &[T] {
        self.point_counts.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.point_counts.as_mut_slice()
    }

    pub fn as_ptr(&self) -> *const T {
        self.point_counts.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.point_counts.as_mut_ptr()
    }

    pub fn covered_bits(&self) -> Vec<bool> {
        self.point_counts.iter().map(|&p| p.is_covered()).collect()
    }

    pub fn covered_counts(&self) -> Vec<u64> {
        self.point_counts.iter().map(|&p| p.as_u64()).collect()
    }
}

impl<T> Hash for Coverage<T>
where
    T: CoveragePoint,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.point_counts.hash(state);
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) enum CoverageKind {
    Bool,
    U8,
    U64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) enum AnyCoverage {
    Bool(Coverage<bool>),
    U8(Coverage<u8>),
    U64(Coverage<u64>),
}

impl AnyCoverage {
    pub fn new(kind: CoverageKind, name: &str) -> Self {
        match kind {
            CoverageKind::Bool => Self::Bool(Coverage::<bool>::new(name)),
            CoverageKind::U8 => Self::U8(Coverage::<u8>::new(name)),
            CoverageKind::U64 => Self::U64(Coverage::<u64>::new(name)),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Bool(c) => c.len(),
            Self::U8(c) => c.len(),
            Self::U64(c) => c.len(),
        }
    }

    pub fn update(&mut self) {
        match self {
            Self::Bool(c) => c.update(),
            Self::U8(c) => c.update(),
            Self::U64(c) => c.update(),
        }
    }

    pub fn covered_bits(&self) -> Vec<bool> {
        match self {
            Self::Bool(c) => c.covered_bits(),
            Self::U8(c) => c.covered_bits(),
            Self::U64(c) => c.covered_bits(),
        }
    }

    pub fn covered_counts(&self) -> Vec<u64> {
        match self {
            Self::Bool(c) => c.covered_counts(),
            Self::U8(c) => c.covered_counts(),
            Self::U64(c) => c.covered_counts(),
        }
    }
}

impl Hash for AnyCoverage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Bool(c) => c.hash(state),
            Self::U8(c) => c.hash(state),
            Self::U64(c) => c.hash(state),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct Coverages {
    covers: HashMap<String, AnyCoverage>,
}

impl Coverages {
    pub fn new(cover_names: &[String]) -> Self {
        let mut covers = HashMap::new();

        for cover_name in cover_names {
            unsafe { set_cover_feedback(&cover_name) };
            let data_size = unsafe { get_cover_data_size() };
            covers.insert(
                cover_name.clone(),
                AnyCoverage::new(
                    if data_size == 8 {
                        CoverageKind::U64
                    } else {
                        CoverageKind::U8
                    },
                    cover_name,
                ),
            );
        }

        Self { covers }
    }

    pub fn len(&self) -> usize {
        self.covers.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &AnyCoverage)> {
        self.covers.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (String, AnyCoverage)> {
        self.covers.into_iter()
    }

    pub fn get(&self, cover_name: &str) -> &AnyCoverage {
        self.covers
            .get(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }

    pub fn get_mut(&mut self, cover_name: &str) -> &mut AnyCoverage {
        self.covers
            .get_mut(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }

    pub fn names(&self) -> Vec<String> {
        self.covers.keys().cloned().collect()
    }

    pub fn update(&mut self, cover_name: &str) {
        self.get_mut(cover_name).update();
    }

    pub fn update_all(&mut self) {
        for cover_name in self.names() {
            self.update(&cover_name);
        }
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
    covers: HashMap<String, Vec<bool>>,
}

impl AccumulatedCoverages {
    fn new(cover_names: &[String]) -> Self {
        let mut covers = cover_names
            .iter()
            .map(|cover_name| (cover_name.clone(), Vec::new()))
            .collect::<HashMap<String, Vec<bool>>>();

        Self { covers }
    }

    fn get(&self, cover_name: &str) -> &Vec<bool> {
        self.covers
            .get(cover_name)
            .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
    }

    fn get_mut(&mut self, cover_name: &str) -> &mut Vec<bool> {
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
        .expect("COVERAGES poisoned mutex")
}

fn accumulated_coverages() -> std::sync::MutexGuard<'static, AccumulatedCoverages> {
    ACCUMULATED_COVERAGES
        .get()
        .expect("cover_init() not called")
        .lock()
        .expect("ACCUMULATED_COVERAGES poisoned mutex")
}

fn get_accumulative_coverage(cover_name: &str) -> f64 {
    let guard = accumulated_coverages();
    let accumulated_cov = guard.get(cover_name);
    let mut covered_num: usize = 0;
    for covered in accumulated_cov.iter() {
        if covered.is_covered() {
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
        set_cover_feedback(cover_name);
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

pub(crate) fn cover_display(cover_name: &str) {
    log::info!(
        "{} Accumulative Coverage:       {:.3}%",
        cover_name,
        get_accumulative_coverage(cover_name)
    )
}

pub(crate) fn all_cover_update() {
    let mut cov_guard = coverages();
    let mut accumulated_cov_guard = accumulated_coverages();
    for cover_name in cov_guard.names() {
        cov_guard.update(&cover_name);

        let cov = cov_guard.get(&cover_name);
        let accumulated_cov = accumulated_cov_guard.get_mut(&cover_name);
        let cover_num = unsafe {
            set_cover_feedback(&cover_name);
            get_cover_number()
        };
        if accumulated_cov.is_empty() {
            accumulated_cov.resize(cover_num as usize, false);
        } else {
            assert_eq!(accumulated_cov.len(), cover_num as usize);
        }
        for (i, covered) in cov.covered_bits().into_iter().enumerate() {
            if covered {
                accumulated_cov[i] = true
            }
        }
    }
}

pub(crate) fn all_cover_display() {
    for cover_name in cover_names() {
        cover_display(&cover_name);
    }
    unsafe { display_uncovered_points() };
}
