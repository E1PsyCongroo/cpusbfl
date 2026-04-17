use core::panic;
/**
 * Copyright (c) 2023 Institute of Computing Technology, Chinese Academy of Sciences
 * xfuzz is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL v2.
 * You may obtain a copy of Mulan PSL v2 at:
 *          http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY KIND,
 * EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT,
 * MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::{Mutex, OnceLock};

use crate::harness::*;

struct Coverage {
    cover_points: Vec<u8>,
    accumulated: Vec<u8>,
}

impl Coverage {
    pub fn new(n_cover: usize) -> Self {
        Self {
            cover_points: vec![0; n_cover],
            accumulated: vec![0; n_cover],
        }
    }

    pub fn len(&self) -> usize {
        self.cover_points.capacity()
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.cover_points.as_ptr().cast_mut()
    }

    pub fn accumulate(&mut self) {
        for (i, covered) in self.cover_points.iter().enumerate() {
            if *covered != 0 as u8 {
                self.accumulated[i] = 1;
            }
        }
    }

    pub fn get_accumulative_coverage(&self) -> f64 {
        let mut covered_num: usize = 0;
        for covered in self.accumulated.iter() {
            if *covered != 0 as u8 {
                covered_num += 1;
            }
        }
        100.0 * covered_num as f64 / self.len() as f64
    }

    pub fn display(&self, name: Option<&str>) {
        // println!("Total Covered Points: {:?}", self.accumulated);
        match name {
            Some(name) => println!(
                "{} Accumulative Coverage:       {:.3}%",
                name,
                self.get_accumulative_coverage()
            ),
            None => println!(
                "Total Coverage:       {:.3}%",
                self.get_accumulative_coverage()
            ),
        }
    }
}

static COVERAGE_NAMES: OnceLock<Vec<String>> = OnceLock::new();
static ICOVERAGE: OnceLock<HashMap<String, Mutex<Coverage>>> = OnceLock::new();

/// Call this once, right after your C test‑bench has told you how many
/// counters are present.
pub(crate) fn cover_init(cover_names: Vec<String>) {
    let mut covers = HashMap::new();
    for cover_name in &cover_names {
        unsafe { set_cover_feedback(CString::new(cover_name.as_bytes()).unwrap().as_ptr()) }
        covers.insert(
            cover_name.clone(),
            Mutex::new(Coverage::new(unsafe { get_cover_number() as usize })),
        );
    }
    // `set` returns Err if it was already initialised; handle that however
    // you prefer (here we just ignore the second call).
    let _ = COVERAGE_NAMES.set(cover_names);
    let _ = ICOVERAGE.set(covers);
}

fn cov(cover_name: &str) -> std::sync::MutexGuard<'static, Coverage> {
    ICOVERAGE
        .get()
        .expect("cover_init() not called")
        .get(cover_name)
        .unwrap_or_else(|| panic!("coverage not found: {}", cover_name))
        .lock()
        .expect("poisoned mutex")
}

pub(crate) fn cover_names() -> &'static Vec<String> {
    COVERAGE_NAMES.get().expect("cover_init() not called")
}

pub(crate) fn cover_len(cover_name: &str) -> usize {
    cov(cover_name).len()
}

pub(crate) fn cover_point_name(cover_name: &str, i: usize) -> String {
    let cover_point_name = unsafe {
        set_cover_feedback(CString::new(cover_name.as_bytes()).unwrap().as_ptr());
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
    let guard = cov(cover_name);
    guard.as_mut_ptr().cast::<u8>()
}

pub(crate) fn cover_update_stats(cover_name: &str) {
    unsafe {
        set_cover_feedback(CString::new(cover_name.as_bytes()).unwrap().as_ptr());
        update_stats(cover_as_mut_ptr(cover_name));
    }
}

pub(crate) fn cover_accumulate(cover_name: &str) {
    cov(cover_name).accumulate()
}

pub(crate) fn cover_display(cover_name: &str) {
    cov(cover_name).display(Some(cover_name))
}

pub(crate) fn all_cover_update_stats() {
    for cover_name in cover_names() {
        cover_update_stats(cover_name);
    }
}

pub(crate) fn all_cover_accumulate() {
    for cover_name in cover_names() {
        cover_accumulate(cover_name);
    }
}

pub(crate) fn all_cover_display() {
    for cover_name in cover_names() {
        cover_display(cover_name);
    }
}
