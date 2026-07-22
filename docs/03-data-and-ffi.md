# 第 3 章：覆盖率、状态跟踪与 FFI

## 3.1 FFI 边界

Rust 在 `harness.rs` 中声明宿主必须实现的 C ABI。宿主可使用 C、C++ 或其他能够导出
兼容符号的语言实现接口。

| C ABI | 方向 | 用途 |
| --- | --- | --- |
| `sim_main(argc, argv)` | Rust → C++ | 执行一次完整仿真 |
| `set_cover_feedback(name)` | Rust → C++ | 切换当前 coverage group |
| `get_cover_number()` | Rust → C++ | 返回当前 group 的 point 数 |
| `get_cover_point_name(i)` | Rust → C++ | 返回 point 的 Verilator metadata 名称 |
| `get_cover_data_size()` | Rust → C++ | 返回单 point 数据宽度 |
| `update_stats_cover(ptr)` | Rust → C++ | 把计数器复制到 Rust 缓冲区 |
| `set_state_feedback(name)` | Rust → C++ | 切换当前 state tracker |
| `get_state_number()` | Rust → C++ | 返回当前序列长度 |
| `update_stats_state(ptr)` | Rust → C++ | 把状态数组复制到 Rust 缓冲区 |

所有名称切换都通过 C 字符串完成。C++ 使用大小写不敏感比较，但 CLI 文档仍使用源码
中的规范名称。

## 3.2 仿真输入传递

LibAFL 操作内存中的 `BytesInput`，当前宿主协议通过 ELF 路径执行程序。桥接层为每次
执行创建临时文件并写入 input bytes，然后构造：

```text
emu -E <temporary-elf> <simulator-extra-args...>
```

临时文件在 `sim_main` 返回后随 `NamedTempFile` 生命周期删除。此设计避免修改用户的
原始 ELF，但每次执行都会产生一次文件写入。

## 3.3 Coverage 数据模型

宿主 coverage collector 应在每次执行结束时：

1. 将 Verilator coverage 写到临时文件；
2. 解析每条 `C '...' count` 记录；
3. 根据 `type` 分成 line、branch、expr、toggle 四个 group；
4. 保存 filename、line、column、type、hierarchy 等 metadata 和 `u64` count；
5. 校验后续执行的 point 数量和顺序与首次执行一致。

Rust `Coverages` 按 CLI 中选择的名称保存多个 `AnyCoverage`。当前 Verilator 实现的
单点宽度是 `u64`，`get_cover_data_size()` 用于在 Rust 侧选择存储类型。一个 point
是否 executed 由 `count != 0` 决定，而原始 count 仍用于 coverage distance。

Point 名称格式来自 Verilator metadata，例如：

```text
filename: ..., lineno: 123, column: 7, type: branch, ..., hier: TOP....
```

后续 RTL block 映射依赖其中的 `lineno` 和 `hier` 字段。

## 3.4 累计 coverage 与 testcase coverage

进程维护两类数据：

- `COVERAGES`：最近一次执行的完整 count vector；
- `ACCUMULATED_COVERAGES`：进程生命周期内各 point 是否至少命中过一次。

`CoveragesObserver` 对最近一次完整 `Coverages` 计算 hash。`NewHashFeedback` 使用该
hash 判断 coverage 组合是否新颖。接受进 corpus 的 testcase 会附加
`CoveragesMetadata`，checkpoint 保存这份 metadata。

Coverage hash 在对 `HashMap` entry 排序后计算，因此不依赖 map 的随机迭代顺序。

## 3.5 状态序列

当前 ABI 定义三种体系结构状态序列：

| 名称 | Rust 类型 | 内容 |
| --- | --- | --- |
| `PCState` | 单个 `u64` | 动态 PC |
| `ArchIntRegState` | `[u64; 32]` | 32 个整数寄存器 |
| `CSRState` | 18 个 `u64` 字段 | privilege mode 及 M/S mode 关键 CSR |

宿主应在退休指令、同步异常等明确的推进点采样状态，并在每次仿真开始时清空上一次
执行的所有序列。

Rust `StateTrackers::len()` 取三个 tracker 的最大长度，并要求 CLI 选中的 tracker
具有同样长度。未知 state 名称会触发 panic。

## 3.6 ABI 内存布局约束

`update_stats_state` 直接使用 `memcpy` 把 C++ vector 数据写入 Rust 预分配数组，因而
两侧布局必须完全一致：

- 字段顺序、字段宽度和数组长度必须相同；
- Rust 聚合状态使用 `#[repr(C)]`；
- 新增或删除 CSR 字段必须同步修改 Rust 和 C++；
- 不能在一侧单独改变 RV32/RV64 扩展存储宽度；
- `get_state_number` 返回值必须与复制的 element 数一致。

这类不一致通常不会产生友好的类型错误，而可能表现为数据错位甚至内存破坏，因此是
接入新 DUT 时最重要的不变量之一。

## 3.7 Tracker window

完整状态序列先由 C++ 收集，再由 `StateTrackersObserver` 截取最后
`tracker_window_size` 个 entry。截取后的序列用于：

- state hash feedback；
- testcase metadata；
- coverage/state distance；
- PSBFL/WitHW 的 mutation candidate trace。

因此 tracker window 同时影响 corpus 新颖性、变异位置和后续 selection。Checkpoint
把这个参数作为兼容配置的一部分，恢复或分析时必须一致。

## 3.8 State hash 与状态距离不是同一概念

Observer 的 state hash 包含 CLI 选择的 tracker，因此 PC 可以影响 testcase 是否
进入 corpus。后续 `state_trackers_distance` 当前只把整数寄存器和 CSR 配对成
`CoreStateRef`，使用 FastDTW 计算距离；PC 不直接进入该距离公式。

这一区别很重要：

- `PCState` 参与新颖性判断和 guided mutation；
- `ArchIntRegState`、`CSRState` 参与 selection/WitHW fitness 的状态距离；
- 改变 `--state` 可能同时改变 hash 行为和可用数据，但不会自动改变距离函数实现。

## 3.9 Feedback metadata

每个被接受 testcase 可携带：

| Metadata | 内容 |
| --- | --- |
| `PassedMetadata` | 本次执行是 `Ok` 还是 `Crash` |
| `CoveragesMetadata` | 完整 coverage counts |
| `StateTrackersMetadata` | window 截取后的状态序列 |
| `MutationMetadata` | 从 parent lineage 累积的 mutated PC 集合 |

WitHW 还把每个 mutation candidate 的动态 priority 保存为 state-level metadata，
以便 checkpoint 恢复自适应学习状态。

## 3.10 安全边界和失败方式

当前 FFI 依赖若干 `unsafe` 假设：

- C++ 返回的 point 数在同一进程中稳定；
- point name 指针在读取期间有效且以 NUL 结尾；
- coverage/state output buffer 类型和长度正确；
- observer 持有的全局对象地址在整个 fuzz session 中不移动；
- `sim_main` 每次调用都能完整重置 Verilator 和统计状态。

扩展接口时应优先在边界处增加数量、版本或 size 校验，而不是把不匹配留给下游 hash
或 SBFL 阶段发现。
