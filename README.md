# CPU SBFL

[中文](README_CN.md) | English

`cpusbfl` is a Rust library for software-driven, spectrum-based fault
localization of processor RTL. Starting from an executable input that exposes
a DUT/reference-model mismatch, it mutates executable instructions, collects
passing counterexamples, and ranks coverage points or parsed RTL blocks by
suspiciousness.

The crate builds as `libcpusbfl.so`. It is not a standalone `cargo run`
application: a host simulator must link the library, provide the C ABI declared
by `src/harness.rs`, and expose coverage and architectural-state observations.
Host-specific build systems, simulator setup, and experiment runners belong to
the host integration rather than this project.

## Features

- `psbfl`, `random`, and `wit-hw` generation strategies;
- coverage- and state-guided LibAFL corpus construction;
- PC, integer-register, and CSR state sequences;
- random, near-failure, and diversity-aware passing-case selection;
- Tarantula, Ochiai, Jaccard, DStar, GP19, Barinel, Crosstab, Zoltar, and
  Ample suspiciousness metrics;
- versioned and checksummed checkpoints for resume and offline analysis;
- optional ELF reduction, coverage reduction, and RTL block mapping.

## Project Layout

```text
src/
├── app/                   # workload, generation, and analysis workflows
├── cli/                   # command-line definitions
├── fuzzer.rs              # LibAFL state, executor, and fuzz loop
├── harness.rs             # host simulator C ABI
├── coverage.rs            # coverage groups and point data
├── state_tracker.rs       # PC/register/CSR sequences
├── observer/              # LibAFL observers
├── feedback/              # coverage/state/passing feedback
├── mutator/               # Random, PSBFL, and WitHW mutators
├── scheduler/             # adaptive scheduling and corpus bounds
├── selection.rs           # Random, Sort, and Diverse selection
├── similarity.rs          # coverage/state distance functions
├── spectrum/              # spectrum matrices and metrics
├── block/                 # SystemVerilog block parsing
├── checkpoint.rs          # checkpoint serialization and validation
└── reduce/                # executable and coverage reduction
```

## Requirements and Build

The Rust project requires a recent Rust toolchain and Cargo. Build the shared
library from this directory:

```bash
cargo build --release
```

The result is normally:

```text
target/release/libcpusbfl.so
```

Building the shared library does not create a runnable simulator. The host must
link it and implement the simulator, coverage, and state callbacks described in
[the FFI chapter](docs/03-data-and-ffi.md).

## Host Interface

At a high level, one input follows this flow:

```text
ELF bytes
  -> LibAFL executor
  -> host sim_main()
  -> DUT/reference comparison
  -> coverage and architectural-state callbacks
  -> corpus feedback
  -> passing-case selection
  -> SBFL ranking
```

The host executable owns its simulator-specific arguments. The library appends
an instruction limit when running generated inputs and treats a nonzero
`sim_main` return value as a failing case.

## CLI Overview

The linked host executable exposes the following command hierarchy:

```text
<sbfl-host> [ROOT OPTIONS] workload   [OPTIONS] -- [WORKLOADS...] [HOST ARGS...]
<sbfl-host> [ROOT OPTIONS] generation [OPTIONS] <psbfl|random|wit-hw> [MODE OPTIONS] -- [HOST ARGS...]
<sbfl-host> [ROOT OPTIONS] analysis   [OPTIONS] -- [HOST ARGS...]
```

Root options such as `--coverage` and `--state` precede the command.
Generation options precede the generation mode, and mode-specific options
follow it. Use `--help` at each level for the authoritative argument list.

### Generate with PSBFL

```bash
"$SBFL_HOST" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --input path/to/failing.elf \
  --output out/psbfl \
  --max-iters 100 \
  --selection diverse \
  --save-corpus out/psbfl.corpus \
  psbfl \
  --mutator-window-size 20 \
  --mutator-weight-strategy uniform \
  -- <host-arguments>
```

The initial input must fail the DUT/reference comparison and must be accepted
into the main corpus by feedback.

### Generate with WitHW

```bash
"$SBFL_HOST" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --input path/to/failing.elf \
  --output out/withw \
  --save-corpus out/withw.corpus \
  wit-hw \
  --max-corpus-size 50 \
  --init-seed-rate 0.2 \
  --mutate-rate 0.2 \
  --priority-alpha 0.1 \
  --failed-reward 5.0 \
  -- <host-arguments>
```

### Resume Generation

`--max-iters` is the number of additional iterations in the resumed run.

```bash
"$SBFL_HOST" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --resume-corpus out/withw.corpus \
  --save-corpus out/withw-next.corpus \
  --max-iters 100 \
  wit-hw \
  -- <host-arguments>
```

### Analyze a Checkpoint

Coverage names, state names, and tracker-window size must match the checkpoint.
Selection, metric, ranking limits, and RTL mapping may be changed.

```bash
"$SBFL_HOST" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  analysis \
  --input out/withw.corpus \
  --output out/reanalysis \
  --tracker-window-size 20 \
  --selection sort \
  --metric barinel \
  --top-pass 10 \
  --top-sus 20 \
  -- <host-arguments>
```

## Output

Depending on the command and options, an output directory may contain:

- `init_case.elf` and selected `rank_*.elf` inputs;
- matching `.cover` and `.state` files with `--save-intermediate`;
- `pass_selection_metrics.csv` with per-pass ranks, distances, and RWMFC;
- `pass_selection_summary.csv` and `fail_point_difficulty.csv` with aggregate
  selection metrics and per-fail-point difficulty;
- `result.log` with coverage-point and optional RTL-block rankings;
- `blocks.json` when RTL mapping is enabled;
- phase timing files;
- optional intermediate reduced ELF files.

Use a fresh output directory. Several artifacts use exclusive-create semantics
and intentionally do not overwrite existing files.

## Technical Documentation

1. [Architecture and execution flow](docs/01-architecture.md)
2. [CLI and workflows](docs/02-cli-and-workflows.md)
3. [Coverage, state tracking, and FFI](docs/03-data-and-ffi.md)
4. [Corpus generation strategies](docs/04-generation-strategies.md)
5. [Selection and SBFL analysis](docs/05-analysis.md)
6. [Checkpoints and artifacts](docs/06-checkpoints-and-artifacts.md)
7. [Development and extension guide](docs/07-development.md)

See the [documentation index](docs/README.md) for suggested reading paths.

## Development Checks

Run from this project directory:

```bash
cargo fmt --all -- --check
RUSTC_WRAPPER= cargo check
RUSTC_WRAPPER= cargo clippy --all-targets --all-features
```

Set `RUST_LOG=debug` or `RUST_LOG=trace` for detailed diagnostics.

## License

This project is licensed under the [Mulan Permissive Software License, Version
2](LICENSE).
