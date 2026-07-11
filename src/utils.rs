use libafl::prelude::*;
use md5;
use std::io::Write;

use crate::coverage::*;
use crate::fuzzer::CaseMetadata;

pub(crate) fn load_initial_case(
    corpus_input: &str,
) -> Result<BytesInput, Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(corpus_input);
    let input_path = if path.is_file() {
        path.clone()
    } else if path.is_dir() {
        let mut entries = std::fs::read_dir(&path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|entry_path| entry_path.is_file())
            .collect::<Vec<_>>();
        entries.sort();
        entries
            .into_iter()
            .next()
            .ok_or_else(|| format!("No testcase found in corpus_input directory {path:?}"))?
    } else {
        return Err(format!("corpus_input {path:?} is neither a file nor a directory").into());
    };

    let bytes = std::fs::read(&input_path)?;

    Ok(BytesInput::new(bytes))
}

pub fn store_testcase(
    input: &BytesInput,
    metadata: Option<&CaseMetadata>,
    output_dir: &str,
    name: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&output_dir)?;

    let filename = match name {
        Some(name) => name.to_string(),
        None => {
            let mut context = md5::Context::new();
            context.consume(input.mutator_bytes());
            format!("{:x}", context.compute())
        }
    };

    input.to_file(std::path::PathBuf::from(format!("{output_dir}/{filename}.elf")).as_path())?;

    if let Some(metadata) = metadata {
        let mut cover_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(std::path::PathBuf::from(format!("{output_dir}/{filename}.cover")).as_path())?;

        for cover_name in cover_names() {
            writeln!(cover_file, "cover points of {cover_name}:")?;
            for (point, count) in metadata
                .covers
                .covered_counts(&cover_name)
                .into_iter()
                .enumerate()
            {
                writeln!(
                    cover_file,
                    "[{}]: \"{}\"({})",
                    point,
                    cover_point_name(&cover_name, point),
                    count
                )?;
            }
        }

        let mut state_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(std::path::PathBuf::from(format!("{output_dir}/{filename}.state")).as_path())?;

        for idx in 0..metadata.state_trackers.len() {
            writeln!(
                state_file,
                "[{idx}]:\n{}{:#}{:#}",
                metadata.state_trackers.pc_tracker.as_slice()[idx],
                metadata.state_trackers.arch_int_reg_tracker.as_slice()[idx],
                metadata.state_trackers.csr_tracker.as_slice()[idx],
            )?;
        }
    }

    Ok(())
}

pub(crate) fn thread_cpu_time_now() -> std::io::Result<std::time::Duration> {
    cpu_time_now(libc::CLOCK_THREAD_CPUTIME_ID)
}

pub(crate) fn process_cpu_time_now() -> std::io::Result<std::time::Duration> {
    cpu_time_now(libc::CLOCK_PROCESS_CPUTIME_ID)
}

fn cpu_time_now(clock_id: libc::clockid_t) -> std::io::Result<std::time::Duration> {
    let mut ts = std::mem::MaybeUninit::<libc::timespec>::uninit();

    let rc = unsafe { libc::clock_gettime(clock_id, ts.as_mut_ptr()) };

    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let ts = unsafe { ts.assume_init() };

    Ok(std::time::Duration::new(
        ts.tv_sec as u64,
        ts.tv_nsec as u32,
    ))
}
