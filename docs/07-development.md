# 第 7 章：开发、验证与扩展

本章给出修改 crate 时最常用的代码入口、验证层级和扩展约束。

## 7.1 模块地图

```text
src/
├── app/                   # workload、generation、analysis 编排
│   └── generation/        # 新建/恢复 session 与模式构造
├── cli/                   # Clap 参数和 ValueEnum
├── fuzzer.rs              # LibAFL state、executor 与 fuzz loop
├── harness.rs             # Rust 到 C++ sim_main 的调用
├── observer/              # coverage/state observer
├── feedback/              # 新覆盖、新状态和 passed metadata
├── mutator/               # Random、PSBFL、WitHW mutation
├── scheduler/             # WitHW seed 调度与 corpus 上界
├── selection.rs           # Random/Sort/Diverse 通过用例选择
├── similarity.rs          # coverage/state 距离和 quantile transform
├── spectrum/              # 频谱统计和 SBFL 指标
├── bugloc.rs              # 排名输出与 RTL block 聚合
├── block/                 # SystemVerilog 解析和 block 映射
├── checkpoint.rs          # checkpoint 编解码、校验和恢复
├── elf.rs / inst.rs       # ELF 地址映射、指令长度与改写
└── reduce/                # 输入与覆盖率约简
```

C++ 集成位于 crate 的相邻目录 `../src/csrc/`，FuseSoC core 文件和构建 hook 位于
`../ibex_simple_system_sbfl.core` 与 `../ibex_sbfl_setup.core`。

## 7.2 本地检查

在 crate 目录可运行：

```bash
cargo fmt --all -- --check
RUSTC_WRAPPER= cargo check
RUSTC_WRAPPER= cargo clippy --all-targets --all-features
```

`RUSTC_WRAPPER=` 用于避免调用者环境中遗留的 wrapper 影响检查；如果本地未设置 wrapper，
可省略。完整集成构建需要 Spike 的 `pkg-config` 文件及 Verilator/FuseSoC 环境，命令见
[README_CN](../README_CN.md#构建)。

该 crate 导出 C ABI `main` 并作为 `cdylib` 链入仿真器。测试配置需要同时考虑 Rust test
harness 与导出入口的符号关系；新增单元测试时，应先确认 `cargo test` 的链接方式，并用
最终 `Vibex_simple_system` 做至少一次集成验证。

建议按风险递增执行：

1. 格式化和静态检查；
2. 针对算法边界的单元测试；
3. 用一个短失败 ELF 做少量 generation iterations；
4. 保存并恢复 checkpoint；
5. 对同一 checkpoint 运行 analysis，检查排名与产物；
6. 执行完整 bugset runner。

## 7.3 新增 coverage 类型

新增 coverage 通常需要同时修改 C++ 和 Rust：

1. 在 `../src/csrc/` 实现或注册 coverage collector；
2. 在仿真运行结束时向 `CosimStats` 暴露稳定的名称、point 数量和计数数组；
3. 在 Rust `coverage.rs` 和 observer 层增加同名分派；
4. 保证 C ABI 的指针生命周期和元素宽度一致；
5. 更新根 CLI 的 supported values、README 和第 3 章；
6. 用已知 workload 检查 point ID 在重复构建/运行间是否稳定。

point 数量或顺序变化会使距离、checkpoint observer 数据和不同实验结果不可直接比较。

## 7.4 新增 state tracker

state tracker 的扩展步骤为：

1. 在 C++ 侧定义采样结构和 tracker，并注册到 `CosimStats`；
2. 在 Rust 定义 `#[repr(C)]` 对应结构，逐字段核对大小、对齐和 signedness；
3. 在 `state_tracker.rs` 增加名称分派、复制和序列化；
4. 明确该 tracker 是否参与 hash、距离计算或只用于轨迹控制；
5. 为零长度、不同长度、完全相同和单字段不同编写距离测试；
6. 同步 CLI 默认值、checkpoint 兼容性和文档。

不要跨 FFI 保存仅在一次仿真调用期间有效的裸指针；Rust observer 应在有效期内复制数据。

## 7.5 新增 generation mode

一个新模式至少涉及：

- 在 `src/cli/` 增加 Clap subcommand/ValueEnum；
- 在 `src/app/generation/strategy.rs` 构造对应 mutator 和 scheduler；
- 定义 seed 选择、mutation 范围、corpus 淘汰和 reward 语义；
- 为 testcase/state metadata 实现序列化；
- 设计恢复时必须一致的 mode 参数，并决定是否纳入 checkpoint config；
- 更新两个 README、第 2/4/6 章以及相关 runner 脚本。

凡是需要随 checkpoint 恢复的 metadata，都必须支持 Serde，并按项目的 LibAFL
`SerdeAny` 注册方式加入 state。仅存在进程内存中的自适应状态会在恢复后静默丢失。

## 7.6 新增 SBFL 指标

新增指标时应：

1. 在 CLI metric enum 加入可解析名称；
2. 在 `src/spectrum/` 实现公式；
3. 明确定义所有零分母和无覆盖情况；
4. 测试 `ef/ep/nf/np` 的典型值与边界值；
5. 确认 `NaN`、正无穷和排序规则不会使排名 panic；
6. 更新 README 的指标列表和[第 5 章](05-analysis.md#58-sbfl-指标)。

## 7.7 修改 Selection::Diverse

Diverse 同时使用两种不同距离，修改时应保持职责分离：

```text
fail_distance(i)      = candidate pass i <-> initial failing case
min_pass_distance(i)  = min(candidate pass i <-> each selected pass)
```

第一项控制“接近失败”，第二项控制“通过用例之间的多样性”。建议至少覆盖以下回归场景：

- 第一项固定为最近失败的 pass，且 `min_pass_distance = 0`；
- 第二项的多样性只相对已经选中的 pass 计算；
- `lambda = 0` 等价于仅按接近失败选择；
- `lambda = 1` 优先最大化 pass-pass 最小距离；
- 相同距离下的 corpus ID tie-break 稳定；
- limit 为 0、1，大于候选数，以及 pool factor 边界。

## 7.8 不变量与常见陷阱

- 初始输入必须失败，且始终保留在 corpus 中；
- 16-bit/32-bit 指令改写必须保持长度，避免破坏后续地址映射；
- coverage point 顺序、state FFI 布局和 CLI 名称都是隐式数据协议；
- 使用多个 coverage/state 时，名称和顺序应在恢复前后完全一致；
- `MutationMetadata` 是 coverage reduction 的输入，不能在 lineage 中意外丢弃；
- RTL 文件扫描当前不递归，且 mapping 依赖文件、层次和行号一致；
- 输出目录中的部分文件拒绝覆盖，重复实验应使用新目录；
- checkpoint 只校验部分通用配置，不能代替完整实验 manifest；
- executor 与仿真器在同一进程，C++ 全局状态必须在每次输入前正确重置；
- Random 模式只修改 `.text`，ELF 重写后的总长度不得超过输入缓冲区。

## 7.9 文档同步清单

CLI 或算法行为变化后，至少检查：

- `README.md` 与 `README_CN.md`；
- `docs/02-cli-and-workflows.md` 及对应算法章节；
- `scripts/run_bugset_sbfl.sh`；
- `scripts/run_bugset_psbfl.sh`；
- `scripts/run_bugset_withw.sh`；
- checkpoint 兼容性说明和已有实验脚本。

