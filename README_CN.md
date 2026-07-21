# 面向 Ibex Simple System 的 CPU SBFL

中文 | [English](README.md)

`cpusbfl` 将差分仿真、软件输入模糊测试和基于频谱的故障定位（SBFL）结合起来，
用于 Ibex 的 Verilator Simple System。它从一个能够复现 Ibex/Spike 差分错误的
ELF 程序出发，通过修改可执行指令收集通过测试，再根据可疑度对 RTL 覆盖点排序，
并可进一步映射到 RTL 数据流块。

Rust crate 会编译成 `libcpusbfl.so` 并链接进 `Vibex_simple_system`。它不是可通过
`cargo run` 独立启动的程序；最终可执行文件需要同时提供仿真器、Spike
协同仿真、覆盖率和状态跟踪所需的回调接口。

## 主要能力

- `psbfl`、`random`、`wit-hw` 三种测试生成模式；
- 支持 Verilator line、branch、expression、toggle 覆盖率；
- 支持 PC、整数寄存器和 CSR 状态序列；
- 基于覆盖率和状态哈希构建 LibAFL corpus；
- 支持随机、近失败排序和多样性优先的通过用例选择；
- 支持 Tarantula、Ochiai、Jaccard、DStar、GP19、Barinel、Crosstab、
  Zoltar、Ample 九种 SBFL 指标；
- 支持带版本号和校验和的 corpus checkpoint，可恢复生成或离线重分析；
- 可选 ELF 约简、覆盖率约简和 RTL block 映射。

## 在仓库中的位置

本目录包含 Rust 实现，相邻目录包含仿真器侧集成：

```text
dv/verilator/simple_system_sbfl/
├── ibex_simple_system_sbfl.core       # FuseSoC/Verilator 集成
├── ibex_sbfl_setup.core               # 环境检查和 Rust 构建 hook
├── src/csrc/                          # 仿真、覆盖率、状态和 Spike 桥接
└── sbfl/                              # 当前 Rust crate
```

主要运行流程为：

```text
ELF 输入
  -> Rust LibAFL executor
  -> C++ 仿真器中的 sim_main()
  -> Ibex/Spike 差分执行
  -> 覆盖率与体系结构状态 observer
  -> 有价值的通过用例 corpus
  -> SBFL 计算
  -> 覆盖点 / RTL block 排名
```

## 环境依赖

集成环境需要：

- Rust 工具链和 Cargo；
- [`cargo-make`](https://github.com/sagiegurari/cargo-make)；
- Ibex 工程要求的 FuseSoC 和 Verilator；
- Ibex co-simulation 版本的 Spike；
- `riscv-riscv`、`riscv-disasm`、`riscv-fdt` 对应的 `pkg-config` 配置。

例如 Spike 安装到 `/opt/spike-cosim` 后：

```bash
export PKG_CONFIG_PATH=/opt/spike-cosim/lib/pkgconfig:${PKG_CONFIG_PATH}
cargo install cargo-make
```

FuseSoC 的 pre-build hook 会在 `IBEX_HOME` 下执行 `cargo make build-all`，因此
`IBEX_HOME` 必须指向 Ibex 仓库根目录。

## 构建

在 Ibex 仓库根目录执行：

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

构建 hook 会生成 `target/release/libcpusbfl.so`，最终可执行文件通常位于：

```text
build/lowrisc_ibex_ibex_simple_system_sbfl_0/sim-verilator/Vibex_simple_system
```

后续示例使用以下变量：

```bash
SBFL_BIN=build/lowrisc_ibex_ibex_simple_system_sbfl_0/sim-verilator/Vibex_simple_system
"$SBFL_BIN" --help
```

## 快速开始

### 1. 使用 PSBFL 生成通过用例

初始 ELF 必须能够复现差分失败。通过的初始用例无法构成失败频谱，因此会被拒绝。

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

### 2. 使用 WitHW 生成通过用例

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

### 3. 恢复生成

恢复运行时，`--max-iters` 表示本次新增的迭代次数。

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

### 4. 从 checkpoint 重新分析

无需重新生成 corpus，即可修改通过用例选择策略、SBFL 指标和 RTL 映射配置。
覆盖率名称、状态名称和 tracker window 必须与 checkpoint 一致。

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

### 5. 仅运行 workload

从 `--` 后开始，首个以 `-` 开头的参数之前均视为 ELF workload，剩余参数传给
仿真器。

```bash
"$SBFL_BIN" workload --repeat 1 -- path/to/program.elf -c 5000000
```

## 命令层级

```text
Vibex_simple_system [根参数] workload   [参数] -- [WORKLOADS...] [仿真器参数...]
Vibex_simple_system [根参数] generation [参数] <psbfl|random|wit-hw> [模式参数] -- [仿真器参数...]
Vibex_simple_system [根参数] analysis   [参数] -- [仿真器参数...]
```

`--coverage`、`--state` 等根参数必须放在 command 前；generation 公共参数放在
generation mode 前；模式专属参数放在 mode 后。各层级的 `--help` 是参数列表的
权威来源。

## 输出产物

指定 `--output DIR` 后，运行过程可能生成：

- `init_case.elf`；使用 `--save-intermediate` 时还会生成 `.cover` 和 `.state`；
- 选中通过用例的 `rank_<rank>_id_<id>_dst_<distance>.elf`；
- 使用 `--save-intermediate` 时对应的 `.cover` 和 `.state`；
- 包含覆盖点和可选 RTL block 排名的 `result.log`；
- 启用 RTL 映射时生成的 `blocks.json`；
- 相应阶段的 `reducing_time.txt`、`fuzzing_time.txt`、`gen_time.txt` 和
  `sbfl_time.txt`；
- 使用 `--reduce-insts --save-reduce` 时保存的中间约简 ELF。

建议使用空的输出目录。部分产物使用排他创建模式，不会覆盖已有文件。

## 技术文档

实现细节按以下章节组织：

1. [总体架构与执行流程](docs/01-architecture.md)
2. [CLI 与工作流](docs/02-cli-and-workflows.md)
3. [覆盖率、状态跟踪与 FFI](docs/03-data-and-ffi.md)
4. [Corpus 生成策略](docs/04-generation-strategies.md)
5. [通过用例选择与 SBFL 分析](docs/05-analysis.md)
6. [Checkpoint 与输出产物](docs/06-checkpoints-and-artifacts.md)
7. [开发与扩展指南](docs/07-development.md)

推荐阅读顺序见[文档索引](docs/README.md)。

## 批量运行

Ibex 仓库的 `scripts/` 下还提供：

- `scripts/run_bugset_psbfl.sh`：对应 `GenerationMode::PSBFL`；
- `scripts/run_bugset_withw.sh`：对应 `GenerationMode::WitHW`；
- `scripts/run_bugset_sbfl.sh`：二者共享的 runner。

在 batch wrapper 中，`--save-corpus` 是布尔开关，每个 case 的 checkpoint 固定保存到
`<case_logdir>/saved_corpus`。

批量运行专属参数请查看各脚本的 `--help`。

## 开发检查

在 Ibex workspace 根目录执行：

```bash
cargo fmt --all -- --check
RUSTC_WRAPPER= cargo check -p cpusbfl
RUSTC_WRAPPER= cargo clippy -p cpusbfl --all-targets
```

设置 `RUST_LOG=debug` 或 `RUST_LOG=trace` 可查看选择、变异和覆盖率细节。

## 许可证

本 crate 使用[木兰宽松许可证第 2 版](LICENSE)。周边 Ibex 集成中的文件仍遵循
各自的上游许可证。
