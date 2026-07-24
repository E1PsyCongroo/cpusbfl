# 第 6 章：Checkpoint 与输出产物

checkpoint 用于恢复长时间 generation，也可作为 analysis 的离线输入。本章说明其边界
和文件语义；格式是项目内部格式，不承诺跨版本长期稳定。

## 6.1 文件布局

checkpoint 的逻辑布局为：

```text
magic:   "SBFLCORPUS\0"
digest:  16-byte MD5(uncompressed payload)
payload: zstd-compressed postcard binary checkpoint
```

当前 payload version 为 1，主要包含：

- coverage 名称字符串；
- state 名称字符串；
- tracker window size；
- 初始失败用例的 corpus ID；
- 累计 `completed_iters`；
- 序列化的 LibAFL fuzzer state、corpus、testcase metadata；
- WitHW 使用的自适应 priority metadata。

保存和加载采用流式 postcard 编解码及 zstd level 3 压缩，不会在内存中构造完整的未压缩
payload。当前 version 1 的二进制格式直接替代了早期 JSON 格式，旧 JSON checkpoint 不再
兼容，需要使用新版本重新生成。

MD5 在这里用于发现文件损坏，不提供安全认证。不要把来自不可信来源的 checkpoint 直接
作为可信实验数据。

## 6.2 原子保存

保存时先在目标文件的父目录创建临时文件，依次写入 magic、digest 和 payload，执行
flush 与 `sync_all` 后再持久化到目标路径。父目录不存在时会自动创建。

这种方式避免正常替换过程中留下半个目标文件，但底层文件系统和异常掉电模型仍会影响
最终持久化保证。

## 6.3 加载与校验

加载过程检查：

1. magic 是否匹配；
2. payload 的 MD5 是否匹配；
3. version 是否受支持；
4. corpus 是否非空；
5. 初始 corpus ID 是否存在，且对应 testcase 仍为失败用例；
6. 运行配置是否兼容。

当前兼容性配置只精确比较以下三项：

```text
coverage string
state string
tracker_window_size
```

字符串按原值比较，因此即使语义相同，名称顺序不同也可能被视为不兼容。generation mode
及其 mode-specific 参数、通过用例选择策略、SBFL metric 和 RTL 配置不属于当前兼容性
记录。恢复 generation 时应由调用者保持生成模式和关键算法参数一致；离线 analysis 则
可以有意改变 selection、metric、top-pass、top-sus 和 RTL 配置。

## 6.4 恢复语义

```text
generation --resume-corpus OLD --save-corpus NEW --max-iters N ...
```

表示从 OLD 恢复并再执行 N 次，而不是运行到总计 N 次。可令 OLD 与 NEW 为同一路径来
更新 checkpoint，但实验管理中通常保留不同版本更便于回退和比较。

`--checkpoint-interval` 按累计迭代数切分保存批次。正常结束时，指定
`--save-corpus` 即会保存最终状态。

## 6.5 输出目录

generation 或 analysis 使用 `--output DIR`。不同选项下可能出现以下文件：

| 文件 | 产生条件 | 含义 |
| --- | --- | --- |
| `init_case.elf` | generation 分析阶段 | 实际用于定位的初始失败 ELF |
| `init_case.cover` / `.state` | `--save-intermediate` | 初始用例 observer 数据 |
| `rank_<rank>_id_<id>.elf` | 选择到通过用例 | 按选择结果导出的 ELF |
| 同名 `.cover` / `.state` | `--save-intermediate` | 对应通过用例 observer 数据 |
| `pass_selection_metrics.csv` | 完成通过用例选择 | 每个 pass 的 rank、corpus ID、distance 和边际/累计 RWMFC |
| `pass_selection_summary.csv` | 完成通过用例选择 | 最终 RWMFC、fail-point reachability 和集合覆盖统计 |
| `fail_point_difficulty.csv` | 完成通过用例选择 | 每个 fail 覆盖点的候选 pass 覆盖频率和难度权重 |
| `result.log` | SBFL 完成 | 覆盖点及可选 RTL block 排名 |
| `blocks.json` | 设置 `--rtl` | 解析后的 RTL block 数据 |
| `reducing_time.txt` | 启用输入约简 | 约简阶段 CPU 时间 |
| `fuzzing_time.txt` | generation | fuzzing 阶段 CPU 时间 |
| `gen_time.txt` | generation | 整体生成阶段 CPU 时间 |
| `sbfl_time.txt` | 分析 | SBFL 阶段 CPU 时间 |

时间文件记录的是进程 CPU 时间，不等同于墙钟时间。

部分文件（包括 `.cover`、`.state`、三个选择指标 CSV 和 `result.log`）使用排他创建，
已有同名文件时会失败；`blocks.json` 和时间文件则可覆盖。为保证实验可复现，建议每次
使用新的空输出目录。

## 6.6 ELF 约简中间文件

使用 `--reduce-insts --save-reduce` 时，输出目录还可能包含：

```text
init_nopped.elf
init_striped_prefix.elf
init_striped_suffix.elf
```

`striped` 是当前文件名中的既有拼写。约简顺序包括：

1. 将未执行的可执行指令替换为等长 NOP，同时验证失败轨迹；
2. 尝试剥离前缀，并插入恢复寄存器、CSR、内存和 privilege context 的代码；
3. 尝试通过跳转到最终失败 PC 并 NOP 被跳过部分来剥离后缀；
4. 每一步都重新仿真验证，失败时缩小范围或回退。

前缀重写依赖可插入代码段的 ELF 布局，当前主要面向项目生成的 RISC-V executable ELF。

## 6.7 实验数据管理建议

- checkpoint 可放在输出目录之外，以免清理输出时误删恢复点；
- 同时记录源码 revision、仿真器构建配置、完整 CLI 和随机 seed；
- 不要手工编辑二进制 payload；读取和写入均通过当前版本程序完成；
- 归档前验证 checkpoint 能加载，并保留 `result.log` 和运行日志；
- 比较实验时保持 coverage/state 名称及其顺序一致。
