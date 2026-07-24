mod app;
mod block;
mod bugloc;
mod checkpoint;
mod cli;
mod coverage;
mod elf;
mod feedback;
mod fuzzer;
mod harness;
mod inst;
mod mutator;
mod observer;
mod reduce;
mod scheduler;
mod selection;
mod similarity;
mod spectrum;
mod state_tracker;
mod utils;

#[unsafe(no_mangle)]
fn main() {
    app::run().expect("sbfl")
}
