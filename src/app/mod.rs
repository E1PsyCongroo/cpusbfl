mod analysis;
mod generation;
mod workload;

use std::{
    fs::File,
    io::Write,
    os::fd::{FromRawFd, RawFd},
    path::Path,
};

use clap::Parser;

use crate::cli::{Cli, Command};

type AppResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn duplicate_fd(fd: RawFd) -> std::io::Result<File> {
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: `dup` returned a new, owned file descriptor above.
    Ok(unsafe { File::from_raw_fd(duplicated) })
}

fn init_logger() -> AppResult {
    // Keep logging attached to the stderr that the process started with. The
    // fuzz harness temporarily redirects fd 2 to /dev/null, and LibAFL exits
    // directly from its timeout signal handler without unwinding that guard.
    // A separately owned fd lets the timeout diagnostics remain visible.
    let log_output = duplicate_fd(libc::STDERR_FILENO)?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(Box::new(log_output)))
        .init();
    Ok(())
}

fn ensure_output_dir(output: Option<impl AsRef<Path>>) -> AppResult {
    if let Some(output_dir) = output {
        std::fs::create_dir_all(output_dir)?;
    }
    Ok(())
}

fn write_elapsed(
    output: Option<impl AsRef<Path>>,
    filename: &str,
    elapsed: std::time::Duration,
) -> AppResult {
    if let Some(output_dir) = output {
        let output_dir = output_dir.as_ref().join("elapsed");
        std::fs::create_dir_all(&output_dir)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output_dir.join(filename))?
            .write_all(format!("{elapsed:?}").as_bytes())?;
    }
    Ok(())
}

pub(crate) fn run() -> AppResult {
    init_logger()?;

    let Cli {
        command,
        coverage,
        state,
    } = Cli::parse();

    match command {
        Command::Workload {
            repeat,
            auto_exit,
            extra_args,
        } => workload::run(coverage, state, repeat, auto_exit, extra_args),
        Command::Generation { gen_mode, gen_args } => {
            generation::run(coverage, state, gen_mode, gen_args)
        }
        Command::Analysis {
            input,
            output,
            common_args,
            sbfl_args,
            extra_args,
        } => analysis::run(
            coverage,
            state,
            input,
            output,
            common_args,
            sbfl_args,
            extra_args,
        ),
    }
}
