extern crate md5;

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

use libafl::inputs::{HasMutatorBytes, ValueInput};
use libafl::prelude::{BytesInput, Corpus, InMemoryCorpus, Input, OnDiskCorpus};
use libafl::state::{HasCorpus, StdState};
use libafl_bolts::rands::RomuDuoJrRand;

use crate::coverage::*;
use crate::fuzzer::CaseMetadata;
use crate::state_tracker::*;

// pub fn store_testcases(
//     state: &mut StdState<
//         InMemoryCorpus<ValueInput<Vec<u8>>>,
//         ValueInput<Vec<u8>>,
//         RomuDuoJrRand,
//         OnDiskCorpus<ValueInput<Vec<u8>>>,
//     >,
//     output_dir: String,
// ) {
//     let corpus = state.corpus();

//     let count = corpus.count();
//     println!("Total corpus count: {count}");

//     for id in corpus.ids() {
//         let testcase: std::cell::RefMut<libafl::prelude::Testcase<BytesInput>> =
//             corpus.get(id).unwrap().borrow_mut();
//         let exec_time = testcase.exec_time().map(|s| s.as_secs()).unwrap_or(0);
//         let scheduled_count = testcase.scheduled_count();
//         let parent_id = if testcase.parent_id().is_some() {
//             usize::from(testcase.parent_id().unwrap()) as i32
//         } else {
//             -1
//         };
//         println!(
//             "Corpus {id}: exec_time {exec_time}, scheduled_count {scheduled_count}, parent_id {parent_id}"
//         );
//         let x = testcase.input().as_ref().unwrap();
//         store_testcase(x, &output_dir, Some(id.to_string()));
//     }
// }

pub fn store_testcase(
    input: &BytesInput,
    metadata: Option<&CaseMetadata>,
    output_dir: &String,
    name: Option<String>,
) {
    fs::create_dir_all(&output_dir).expect("Unable to create the output directory");

    let filename = if name.is_some() {
        name.unwrap()
    } else {
        let mut context = md5::Context::new();
        context.consume(input.mutator_bytes());
        format!("{:x}", context.compute())
    };

    input
        .to_file(PathBuf::from(format!("{output_dir}/{filename}.bin")).as_path())
        .expect(format!("written {filename} failed").as_str());

    if let Some(metadata) = metadata {
        let mut cover_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(PathBuf::from(format!("{output_dir}/{filename}.cover")).as_path())
            .unwrap();

        for cover_name in cover_names() {
            writeln!(cover_file, "cover points of {cover_name}:").unwrap();
            for (point, count) in metadata
                .covers
                .get(&cover_name)
                .covered_counts()
                .into_iter()
                .enumerate()
            {
                writeln!(
                    cover_file,
                    "[{}]: \"{}\"({})",
                    point,
                    cover_point_name(&cover_name, point),
                    count
                )
                .unwrap();
            }
        }

        let mut state_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(PathBuf::from(format!("{output_dir}/{filename}.state")).as_path())
            .unwrap();

        for (idx, state) in metadata.state_track.iter().enumerate() {
            writeln!(
                state_file,
                "[{idx}]: {}",
                state
                    .as_slice()
                    .iter()
                    .map(|s| format!("{:02x}", s))
                    .collect::<String>()
            )
            .unwrap();
        }
    }
}
