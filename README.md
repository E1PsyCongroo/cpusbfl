# CPU SBFL for the Ibex Simple System

[中文](README_CN.md) | English

`cpusbfl` combines differential simulation, software-input fuzzing, and
spectrum-based fault localization (SBFL) for the Ibex Verilator simple system.
It starts from an ELF program that reproduces an Ibex/Spike mismatch, mutates
executable instructions to collect passing counterexamples, and ranks
instrumented RTL coverage points—and optionally RTL data-flow blocks—by
suspiciousness.

The Rust crate is built as `libcpusbfl.so` and linked into
`Vibex_simple_system`. It is not a standalone `cargo run` application: the
final executable provides the simulator, Spike co-simulation, coverage, and
state-tracking callbacks required by the Rust library.

## Highlights

- Three generation modes: `psbfl`, `random`, and `wit-hw`.
- Verilator line, branch, expression, and toggle coverage support.
- PC, integer-register, and CSR state-sequence tracking.
- Coverage- and state-guided LibAFL corpus construction.
- Random, near-failure sort, and diversity-aware passing-case selection.
- Nine SBFL metrics: Tarantula, Ochiai, Jaccard, DStar, GP19, Barinel,
  Crosstab, Zoltar, and Ample.
- Versioned, checksummed corpus checkpoints for resume and offline analysis.
- Optional ELF reduction, coverage reduction, and RTL block mapping.

## Repository Context

This directory contains the Rust part of the integration. The adjacent
directory contains the simulator-side implementation:

```text
dv/verilator/simple_system_sbfl/
├── ibex_simple_system_sbfl.core       # FuseSoC/Verilator integration
├── ibex_sbfl_setup.core               # setup and Rust build hooks
├── src/csrc/                          # simulator, coverage, state, Spike bridge
└── sbfl/                              # this Rust crate
```

The main runtime flow is:

```text
ELF input
  -> Rust LibAFL executor
  -> sim_main() in the C++ simulator
  -> Ibex/Spike differential execution
  -> coverage and architectural-state observers
  -> interesting passing corpus
  -> SBFL scoring
  -> coverage-point / RTL-block ranking
```

## Prerequisites

The integration requires:

- a Rust toolchain with Cargo;
- [`cargo-make`](https://github.com/sagiegurari/cargo-make);
- FuseSoC and Verilator as required by the Ibex project;
- the Ibex co-simulation build of Spike;
- `pkg-config` entries for `riscv-riscv`, `riscv-disasm`, and `riscv-fdt`.

For example, after installing the Ibex co-simulation Spike under
`/opt/spike-cosim`:

```bash
export PKG_CONFIG_PATH=/opt/spike-cosim/lib/pkgconfig:${PKG_CONFIG_PATH}
cargo install cargo-make
```

The FuseSoC pre-build hook invokes `cargo make build-all` from `IBEX_HOME`, so
set `IBEX_HOME` to the Ibex repository root.

## Build

From the Ibex repository root:

```bash
export IBEX_HOME="$PWD"

fusesoc --cores-root=. run \
  --target=sim \
  --setup \
  --build \
  lowrisc:ibex:ibex_simple_system_sbfl \
  --RV32E=0 \
  --RV32M=ibex_pkg::RV32MFast
```

The build hook produces `target/release/libcpusbfl.so`; the final executable is
normally:

```text
build/lowrisc_ibex_ibex_simple_system_sbfl_0/sim-verilator/Vibex_simple_system
```

Set a convenience variable for the examples below:

```bash
SBFL_BIN=build/lowrisc_ibex_ibex_simple_system_sbfl_0/sim-verilator/Vibex_simple_system
"$SBFL_BIN" --help
```

## Quick Start

### 1. Generate passing cases with PSBFL

The initial ELF must reproduce a differential failure. A passing initial case
is rejected because it cannot serve as the failed spectrum.

```bash
"$SBFL_BIN" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --input path/to/failing.elf \
  --output out/psbfl \
  --max-iters 100 \
  --max-run-timeout 10 \
  --top-pass 10 \
  --selection diverse \
  --metric ochiai \
  psbfl \
  --mutator-window-size 20 \
  --mutator-weight-strategy uniform \
  -- -c 5000000
```

### 2. Generate passing cases with WitHW

```bash
"$SBFL_BIN" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --input path/to/failing.elf \
  --output out/withw \
  --save-corpus out/withw.corpus \
  --checkpoint-interval 25 \
  --selection diverse \
  wit-hw \
  --max-corpus-size 50 \
  --init-seed-rate 0.2 \
  --mutate-rate 0.2 \
  --priority-alpha 0.1 \
  --failed-reward 5.0 \
  -- -c 5000000
```

### 3. Resume generation

`--max-iters` is the number of additional iterations in the resumed run.

```bash
"$SBFL_BIN" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --resume-corpus out/withw.corpus \
  --save-corpus out/withw-next.corpus \
  --max-iters 100 \
  wit-hw \
  -- -c 5000000
```

### 4. Re-run analysis from a checkpoint

Analysis can change case selection, SBFL metric, and RTL mapping without
regenerating the corpus. Coverage names, state names, and tracker window size
must match the saved checkpoint.

```bash
"$SBFL_BIN" \
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
  -- -c 5000000
```

### 5. Run workloads without fuzzing

Arguments before the first hyphen-prefixed argument are treated as ELF
workloads; the remaining values are forwarded to the simulator.

```bash
"$SBFL_BIN" workload --repeat 1 -- path/to/program.elf -c 5000000
```

## Command Layout

```text
Vibex_simple_system [ROOT OPTIONS] workload   [OPTIONS] -- [WORKLOADS...] [SIM ARGS...]
Vibex_simple_system [ROOT OPTIONS] generation [OPTIONS] <psbfl|random|wit-hw> [MODE OPTIONS] -- [SIM ARGS...]
Vibex_simple_system [ROOT OPTIONS] analysis   [OPTIONS] -- [SIM ARGS...]
```

Root options such as `--coverage` and `--state` must precede the command.
Generation options must precede the generation mode; mode-specific options
must follow it. Run `--help` at each level for the authoritative option list.

## Output

With `--output DIR`, the run may create:

- `init_case.elf`, plus `.cover` and `.state` with `--save-intermediate`;
- `rank_<rank>_id_<id>_dst_<distance>.elf` for selected passing cases;
- matching `.cover` and `.state` files with `--save-intermediate`;
- `result.log` with coverage-point and optional RTL-block rankings;
- `blocks.json` when RTL mapping is enabled;
- `reducing_time.txt`, `fuzzing_time.txt`, `gen_time.txt`, and
  `sbfl_time.txt` when their corresponding phases run;
- optional reduced initial ELFs when `--reduce-insts --save-reduce` is used.

Use a fresh output directory. Several artifacts are intentionally created with
exclusive-create semantics and will not overwrite an existing file.

## Technical Documentation

The detailed implementation documentation is organized by chapter:

1. [Architecture and execution flow](docs/01-architecture.md)
2. [CLI and workflows](docs/02-cli-and-workflows.md)
3. [Coverage, state tracking, and FFI](docs/03-data-and-ffi.md)
4. [Corpus generation strategies](docs/04-generation-strategies.md)
5. [Selection and SBFL analysis](docs/05-analysis.md)
6. [Checkpoints and artifacts](docs/06-checkpoints-and-artifacts.md)
7. [Development and extension guide](docs/07-development.md)

See the [documentation index](docs/README.md) for suggested reading paths.

## Batch Runs

The Ibex repository also provides wrappers under `scripts/`:

- `scripts/run_bugset_psbfl.sh` for `GenerationMode::PSBFL`;
- `scripts/run_bugset_withw.sh` for `GenerationMode::WitHW`;
- `scripts/run_bugset_sbfl.sh` as their shared runner.

For the batch wrappers, `--save-corpus` is a boolean flag. Each case writes its
checkpoint to `<case_logdir>/saved_corpus`.

Use each wrapper's `--help` for batch-specific options.

## Development Checks

From the Ibex workspace root:

```bash
cargo fmt --all -- --check
RUSTC_WRAPPER= cargo check -p cpusbfl
RUSTC_WRAPPER= cargo clippy -p cpusbfl --all-targets
```

Set `RUST_LOG=debug` or `RUST_LOG=trace` for detailed selection, mutation, and
coverage diagnostics.

## License

This crate is licensed under the [Mulan Permissive Software License, Version
2](LICENSE). The surrounding Ibex integration also contains files under their
respective upstream licenses.
