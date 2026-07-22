# CPU SBFL

中文 | [English](README.md)

`cpusbfl` 是一个面向处理器 RTL 软件驱动故障定位的 Rust 库。它从能够暴露
DUT/参考模型差异的可执行输入开始，通过修改可执行指令收集通过反例，再根据频谱可疑度
对覆盖点或解析后的 RTL block 排序。

crate 编译为 `libcpusbfl.so`，不能通过 `cargo run` 独立运行。宿主仿真器需要链接该
动态库，实现 `src/harness.rs` 声明的 C ABI，并提供覆盖率与体系结构状态数据。具体
DUT、仿真器构建系统和实验 runner 属于宿主集成，不属于本项目文档。

## 主要能力

- `psbfl`、`random`、`wit-hw` 三种生成策略；
- 基于覆盖率和状态的 LibAFL corpus 构建；
- PC、整数寄存器和 CSR 状态序列；
- 随机、近失败和多样性优先的通过用例选择；
- Tarantula、Ochiai、Jaccard、DStar、GP19、Barinel、Crosstab、Zoltar、
  Ample 九种可疑度指标；
- 支持恢复和离线分析的带版本号、校验和 checkpoint；
- 可选 ELF 约简、覆盖率约简和 RTL block 映射。

## 项目结构

```text
src/
├── app/                   # workload、generation、analysis 工作流
├── cli/                   # 命令行定义
├── fuzzer.rs              # LibAFL state、executor 和 fuzz loop
├── harness.rs             # 宿主仿真器 C ABI
├── coverage.rs            # coverage group 和 point 数据
├── state_tracker.rs       # PC/寄存器/CSR 状态序列
├── observer/              # LibAFL observer
├── feedback/              # coverage/state/passing feedback
├── mutator/               # Random、PSBFL、WitHW mutator
├── scheduler/             # 自适应调度和 corpus 上界
├── selection.rs           # Random、Sort、Diverse 选择
├── similarity.rs          # coverage/state 距离
├── spectrum/              # 频谱矩阵和指标
├── block/                 # SystemVerilog block 解析
├── checkpoint.rs          # checkpoint 序列化与校验
└── reduce/                # 可执行输入和覆盖率约简
```

## 依赖与构建

本项目需要较新的 Rust 工具链和 Cargo。在项目目录执行：

```bash
cargo build --release
```

默认生成：

```text
target/release/libcpusbfl.so
```

构建动态库不会产生可直接运行的仿真器。宿主必须链接该动态库，并实现
[FFI 章节](docs/03-data-and-ffi.md)描述的仿真、覆盖率和状态回调。

## 宿主接口

单个输入的高层流程为：

```text
ELF bytes
  -> LibAFL executor
  -> 宿主 sim_main()
  -> DUT/参考模型比较
  -> coverage 和体系结构状态回调
  -> corpus feedback
  -> 通过用例选择
  -> SBFL 排名
```

宿主可定义自己的仿真器参数。生成输入运行时，库会追加指令数限制，并把
`sim_main` 的非零返回值视为失败用例。

## CLI 概览

链接后的宿主 executable 提供以下命令层级：

```text
<sbfl-host> [根参数] workload   [参数] -- [WORKLOADS...] [宿主参数...]
<sbfl-host> [根参数] generation [参数] <psbfl|random|wit-hw> [模式参数] -- [宿主参数...]
<sbfl-host> [根参数] analysis   [参数] -- [宿主参数...]
```

`--coverage`、`--state` 等根参数位于 command 前；generation 公共参数位于 mode
前；模式参数位于 mode 后。各层级的 `--help` 是权威参数列表。

### 使用 PSBFL 生成

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
  -- <宿主参数>
```

初始输入必须触发 DUT/参考模型比较失败，并被 feedback 接收到主 corpus。

### 使用 WitHW 生成

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
  -- <宿主参数>
```

### 恢复生成

恢复运行时，`--max-iters` 表示本次新增的迭代次数。

```bash
"$SBFL_HOST" \
  --coverage verilator.branch,verilator.line \
  --state PCState,ArchIntRegState,CSRState \
  generation \
  --resume-corpus out/withw.corpus \
  --save-corpus out/withw-next.corpus \
  --max-iters 100 \
  wit-hw \
  -- <宿主参数>
```

### 分析 checkpoint

coverage 名称、state 名称和 tracker window 必须与 checkpoint 一致；selection、
metric、排名数量和 RTL 映射可以改变。

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
  -- <宿主参数>
```

## 输出产物

根据命令和选项，输出目录可能包含：

- `init_case.elf` 和选中的 `rank_*.elf`；
- 使用 `--save-intermediate` 时生成的 `.cover` 和 `.state`；
- 覆盖点和可选 RTL block 排名 `result.log`；
- 启用 RTL 映射时生成的 `blocks.json`；
- 各阶段计时文件；
- 可选的 ELF 约简中间文件。

建议使用新的空输出目录。部分产物采用排他创建，不会覆盖同名文件。

## 技术文档

1. [总体架构与执行流程](docs/01-architecture.md)
2. [CLI 与工作流](docs/02-cli-and-workflows.md)
3. [覆盖率、状态跟踪与 FFI](docs/03-data-and-ffi.md)
4. [Corpus 生成策略](docs/04-generation-strategies.md)
5. [通过用例选择与 SBFL 分析](docs/05-analysis.md)
6. [Checkpoint 与输出产物](docs/06-checkpoints-and-artifacts.md)
7. [开发与扩展指南](docs/07-development.md)

推荐阅读路径见[文档索引](docs/README.md)。

## 开发检查

在本项目目录执行：

```bash
cargo fmt --all -- --check
RUSTC_WRAPPER= cargo check
RUSTC_WRAPPER= cargo clippy --all-targets --all-features
```

设置 `RUST_LOG=debug` 或 `RUST_LOG=trace` 可查看详细诊断。

## 许可证

本项目使用[木兰宽松许可证第 2 版](LICENSE)。
