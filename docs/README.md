# CPU SBFL 技术文档

本目录描述 `dv/verilator/simple_system_sbfl/sbfl` 的当前实现。README 负责快速上手，
这里侧重模块边界、数据模型、算法、不变量和扩展点。

## 章节大纲

| 章节 | 内容 | 适合读者 |
| --- | --- | --- |
| [01 总体架构](01-architecture.md) | Rust/C++/Verilator/Spike 的链接关系、运行时控制流和生命周期 | 首次接触项目的开发者 |
| [02 CLI 与工作流](02-cli-and-workflows.md) | 命令层级、参数约束、generation/resume/analysis/workload 流程 | 使用者、实验脚本作者 |
| [03 数据与 FFI](03-data-and-ffi.md) | 覆盖率、状态序列、observer、C ABI 和内存布局约束 | 仿真器集成开发者 |
| [04 生成策略](04-generation-strategies.md) | LibAFL corpus、Random、PSBFL、WitHW、调度和优先级更新 | 模糊测试算法开发者 |
| [05 分析算法](05-analysis.md) | 距离归一化、通过用例选择、频谱矩阵、SBFL 指标和 RTL block 映射 | 故障定位算法开发者 |
| [06 Checkpoint 与产物](06-checkpoints-and-artifacts.md) | 文件格式、恢复语义、兼容性和输出目录内容 | 长时间实验维护者 |
| [07 开发与扩展](07-development.md) | 模块地图、构建检查、新增覆盖率/状态/策略/指标的方法 | 贡献者 |

## 推荐阅读路径

- **只想运行实验**：README → 第 2 章 → 第 6 章。
- **修改生成算法**：第 1 章 → 第 3 章 → 第 4 章。
- **修改 fault localization**：第 3 章 → 第 5 章。
- **接入新的仿真器或 DUT**：第 1 章 → 第 3 章 → 第 7 章。

## 文档与代码的对应关系

| 主题 | 主要源码 |
| --- | --- |
| 顶层命令分派 | `src/app/`, `src/cli/` |
| LibAFL 执行循环 | `src/fuzzer.rs` |
| 仿真器桥接 | `src/harness.rs`, `../src/csrc/` |
| 覆盖率和状态 | `src/coverage.rs`, `src/state_tracker.rs`, `src/observer/`, `src/feedback/` |
| ELF 和指令处理 | `src/elf.rs`, `src/inst.rs`, `src/reduce/` |
| 生成策略 | `src/mutator/`, `src/scheduler/`, `src/app/generation/strategy.rs` |
| 通过用例选择 | `src/selection.rs`, `src/similarity.rs` |
| SBFL 和 RTL 映射 | `src/spectrum/`, `src/bugloc.rs`, `src/block/` |
| Corpus checkpoint | `src/checkpoint.rs` |

文档以当前源码为准。CLI 发生变更时，应同时更新两个 README、第 2 章以及仓库根目录
下的 bugset runner 脚本。
