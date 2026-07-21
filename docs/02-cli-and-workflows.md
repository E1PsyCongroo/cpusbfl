# 第 2 章：CLI 与工作流

## 2.1 命令语法

最终 executable 的命令层级为：

```text
Vibex_simple_system [ROOT OPTIONS] <COMMAND>

COMMAND := workload | generation | analysis
generation := generation [GENERATION OPTIONS] <psbfl|random|wit-hw> [MODE OPTIONS]
```

参数位置很重要：

- 根参数放在 command 之前；
- generation 公共参数放在 mode 之前；
- mode 专属参数放在 `psbfl`、`random` 或 `wit-hw` 之后；
- `--` 之后的值由应用自行拆分或透传。

应在实际构建产物上查看帮助：

```bash
"$SBFL_BIN" --help
"$SBFL_BIN" generation --help
"$SBFL_BIN" generation psbfl --help
"$SBFL_BIN" generation wit-hw --help
"$SBFL_BIN" analysis --help
```

## 2.2 根参数

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `-c, --coverage` | `verilator.branch,verilator.line` | 逗号分隔的 coverage group |
| `-s, --state` | `PCState,ArchIntRegState,CSRState` | 逗号分隔的状态序列 |

当前 C++ 集成支持 `verilator.line`、`verilator.branch`、`verilator.expr`、
`verilator.toggle`。状态名称必须是 `PCState`、`ArchIntRegState`、`CSRState`。

## 2.3 Workload

`workload` 用于不变异输入地重复运行一个或多个 ELF，并输出累计覆盖率。

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `--repeat` | `1` | 重复整个 workload 列表的次数 |
| `--auto-exit` | false | 某个 workload 非零退出时立即停止 |

`--` 后的参数按“第一个以 `-` 开头的值”切分：此前是 workload，之后是模拟器参数。

```bash
"$SBFL_BIN" \
  --coverage verilator.branch \
  workload --repeat 3 -- a.elf b.elf -c 5000000
```

该例每轮依次运行 `a.elf`、`b.elf`，并把 `-c 5000000` 传给模拟器。

## 2.4 Generation 公共参数

| 参数 | 缺省值 | 约束或语义 |
| --- | --- | --- |
| `--input PATH` | `corpus` | 新 session 的 ELF 文件或目录；与 resume 冲突 |
| `-r, --reduce-insts` | false | generation 前约简失败 ELF；别名 `--reduce` |
| `--save-reduce` | false | 保存中间约简 ELF；要求 `--reduce-insts` |
| `--resume-corpus FILE` | 无 | 恢复 checkpoint；与 input/reduce 冲突 |
| `--save-corpus FILE` | 无 | 结束时保存 checkpoint |
| `--checkpoint-interval N` | 无 | 每 N 个累计 iteration 保存；要求 save-corpus，N > 0 |
| `--gen-only` | false | 只生成，不执行 selection 和 SBFL |
| `--output PATH` | 无 | 输出 ELF、日志和耗时文件的目录 |
| `--max-iters N` | `100` | 本次运行的 iteration 数，N > 0 |
| `--max-run-timeout N` | `10` | 单个输入的超时秒数，N > 0 |
| `--tracker-window-size N` | `20` | observer 保存的 trace 尾部长度，N > 0 |
| `--cover-distance-weight W` | `0.5` | coverage/state 距离权重，范围 `[0,1]` |
| `--save-intermediate` | false | 为输出 ELF 同时保存 `.cover` 和 `.state` |

如果 `--input` 指向目录，加载器只选取按路径排序后的第一个普通文件；它不是初始
seed corpus 的批量导入接口。

## 2.5 SBFL 和 RTL 参数

这些参数同时存在于 `generation` 和 `analysis`：

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `--top-pass N` | `10` | 最多选取的通过用例数 |
| `--selection` | `sort` | `random`、`sort` 或 `diverse` |
| `--selection-diversity-weight W` | `0.4` | Diverse 中多样性项权重 |
| `--selection-pool-factor N` | `3` | Diverse 候选池相对 top-pass 的倍数 |
| `--reduce-cover` | false | 通过额外仿真削减公共 coverage |
| `--top-sus N` | `10` | 输出排名的覆盖点/RTL block 数 |
| `--metric` | `ochiai` | SBFL 指标 |
| `--rtl-path PATH` | 无 | RTL 文件或只含直接 `.v/.sv` 文件的目录 |
| `--include-paths PATHS` | 无 | 逗号分隔的 SystemVerilog include path |
| `--top-module NAME` | 无 | RTL 顶层 module 名 |
| `--top-scope SCOPE` | 无 | coverage metadata 使用的顶层层次路径 |

运行时 RTL 映射要求 `rtl-path`、`include-paths`、`top-module`、`top-scope` 四项
全部提供，或者全部省略。目录扫描当前不递归，子目录中的 RTL 文件需要显式组织到
扫描目录或作为文件单独传入。

支持的 metric 值为：

```text
tarantula ochiai jaccard dstar gp19 barinel crosstab zoltar ample
```

## 2.6 Mode 专属参数

### PSBFL

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `--mutator-window-size N` | `20` | 从初始动态 PC trace 尾部取候选的窗口 |
| `--mutator-weight-strategy` | `uniform` | PC 候选的采样权重 |

权重策略：

```text
uniform tail_linear tail_quad head_linear head_quad
```

### Random

无 mode 专属参数，使用 LibAFL havoc mutations，但变异范围限制在 ELF `.text`。

### WitHW

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `--max-corpus-size N` | `50` | 保留的通过用例上限 |
| `--init-seed-rate W` | `0.2` | 调度初始失败 seed 的概率 |
| `--mutate-rate W` | `0.2` | 每次从候选 PC 中选择的比例 |
| `--priority-alpha W` | `0.1` | mutation priority 指数更新系数 |
| `--failed-reward V` | `5.0` | 新接受失败用例给予的 reward |

三个 rate/alpha 参数范围为 `[0,1]`；failed reward 必须是有限非负数。

## 2.7 Analysis

`analysis --input FILE` 的 input 是 SBFL checkpoint，不是 ELF。主要用途包括：

- 用不同 metric 重算排名；
- 改变 `top-pass` 和 selection；
- 添加或调整 RTL block 映射；
- 在已有 corpus 上启用 coverage reduction。

Checkpoint 保存的 coverage、state、tracker window 必须和分析命令一致，否则加载被
拒绝。Mode-specific generation 参数不参与 analysis。

## 2.8 模拟器参数

Generation mode 和 analysis 都声明了 trailing `extra_args`。应用只使用从首个
hyphen-prefixed value 开始的部分作为模拟器参数。推荐始终显式使用 `--`：

```bash
... wit-hw -- -c 5000000
... analysis ... -- -c 5000000
```

Fuzzer 在仿真参数中还会追加 `-I <max-inst>`，其中 `max-inst` 来自当前 state tracker
长度，用于限制后续变异程序的执行步数。

## 2.9 Batch runner

Ibex 仓库根目录的脚本在隔离 workdir 中为 bugset 应用 `.sv.diff`、重新构建模拟器并
执行 generation：

- `run_bugset_psbfl.sh` 固定选择 `psbfl`；
- `run_bugset_withw.sh` 固定选择 `wit-hw`；
- `run_bugset_sbfl.sh` 负责共享校验、构建、日志和并发控制。

脚本参数与当前 CLI 对齐，但 `--logs`、`--tmp`、`--jobs` 等属于 batch runner，
不是 SBFL executable 参数。wrapper 的 `--input` 会先解析为绝对路径再传给 executable；
wrapper 的 `--save-corpus` 是不带值的布尔开关，并把每个 case 的 checkpoint 保存为
`<case_logdir>/saved_corpus`。这与 executable 自身需要
`--save-corpus <FILE>` 的接口不同。

## 2.10 常见参数错误

- 把 `--coverage` 放在 `generation` 后：根参数位置错误。
- 把 `--max-iters` 放在 `psbfl` 后：generation 参数被当成 mode 参数。
- 同时使用 `--input` 和 `--resume-corpus`：Clap conflict。
- 单独使用 `--save-reduce`：缺少 `--reduce-insts`。
- 使用 `--checkpoint-interval` 但没有 `--save-corpus`。
- 只提供部分 RTL 参数：Clap 或运行时校验失败。
- 用 passing ELF 作为初始 input：初始失败检查失败。
