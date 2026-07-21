# 第 4 章：测试生成策略

本章描述 generation 阶段如何从一个失败 ELF 产生通过用例。三种模式共用同一套
executor、observer、feedback 和 checkpoint 机制，区别主要在 scheduler 和 mutator。

## 4.1 Corpus 与反馈

运行时状态以 LibAFL `StdState` 保存，输入和 corpus 类型如下：

```text
BytesInput
  -> InMemoryCorpus（待调度用例）
  -> InMemoryCorpus（objective corpus，当前不接收用例）
```

每个输入运行后，`PassedFeedback` 记录该用例是否通过差分检查；用例是否进入主
corpus，则由以下“新颖性”条件决定：

```text
new coverage OR new non-empty state hash
```

因此，“通过”是用例元数据，不等同于“进入 corpus”。失败但产生新覆盖率或新状态的
输入也可能被保留。objective feedback 当前恒为 false。

初始 ELF 有两个重要约束：

1. 它必须触发差分失败；通过的初始输入会被拒绝。
2. 它必须被反馈机制接收到 corpus，否则生成过程无法建立初始失败基准。

executor 在当前进程中调用仿真器，并为每个输入应用 `--max-run-timeout`。每次运行时
传给仿真器的最大指令数会根据 tracker 长度附加为 `-I <max_inst>`。

## 4.2 通用循环与 checkpoint

未启用定期 checkpoint 时，生成器直接运行指定次数。设置
`--checkpoint-interval N` 后，循环以至多 N 次为一个批次，并在每批完成后保存状态。
`completed_iters` 是累计值；从 checkpoint 恢复时，`--max-iters` 表示本次继续执行的
新增次数，而不是全局终点。

无论是否恰好落在 interval 边界，只要指定了 `--save-corpus`，正常结束时都会再保存
一次最终 checkpoint。

## 4.3 Random

Random 模式采用队列调度和 LibAFL havoc mutation，但 mutation 只作用于 ELF 的
`.text` section：

- 解析 ELF，并把 `.text` 内容作为 mutation 缓冲区；
- mutation 后的 `.text` 不得比初始 `.text` 更长；
- 将修改后的 section 写回 ELF；
- 序列化结果不得超过输入缓冲区，剩余字节以零填充。

这种策略简单且覆盖面广，但随机字节不保证是合法或有意义的 RISC-V 指令。它适合作为
基线，不利用失败轨迹中的结构信息。

## 4.4 PSBFL

PSBFL 使用初始失败轨迹构造候选指令集合，并按位置权重反复修改。候选构造过程为：

1. 按动态执行顺序保留每个 PC 的第一次出现；
2. 只取末尾 `--mutator-window-size` 个候选；
3. 将虚拟地址映射到 ELF 文件偏移；
4. 根据指令低位区分 16-bit compressed 指令和 32-bit 指令。

每次 mutation 的尝试次数为：

```text
max(1, floor(sqrt(candidate_count)))
```

被选中的指令会替换为同长度随机字节。`--mutator-weight-strategy` 控制候选权重：

| 策略 | 第 `i` 个候选的权重，候选数为 `n` |
| --- | --- |
| `uniform` | `1` |
| `tail_linear` | `i + 1` |
| `tail_quad` | `(i + 1)^2` |
| `head_linear` | `n - i` |
| `head_quad` | `(n - i)^2` |

其中候选顺序与动态轨迹顺序一致，tail 策略更偏向故障发生前的后部指令，head 策略更
偏向窗口前部。

每个生成输入还会记录 `MutationMetadata`。其中的 PC 集合沿父子 lineage 累积，供后续
通过用例覆盖率约简定位首个受 mutation 影响的位置。

## 4.5 WitHW

WitHW 同时学习候选指令的优先级，并限制 corpus 中通过用例的数量。

### 候选与 mutation 数量

候选来自初始轨迹的最后 `--tracker-window-size` 条记录，再在该窗口内按 PC 去重。一次
mutation 修改的候选数量为：

```text
max(1, floor(candidate_count * mutate_rate))
```

候选按当前 priority 加权、无放回采样，初始 priority 均为 1。

### Seed 调度

scheduler 以 `--init-seed-rate` 的概率直接选择初始失败用例。否则，它计算每个通过用例
相对初始失败用例的覆盖率和状态距离，分别做分位数归一化后合并：

```text
distance = cover_weight * coverage_distance
         + (1 - cover_weight) * state_distance
fitness  = 1 / (1 + distance)
```

因此更接近初始失败的通过用例具有更高 fitness。这里的距离用于选择下一轮 seed；它与
最终 `Selection::Diverse` 中通过用例之间的距离不是同一个计算阶段。

### Reward 与 priority 更新

新接收的通过用例，其 reward 是它到 corpus 中其他通过用例的原始组合距离平均值；如果
还没有其他通过用例，reward 为 1。新接收的失败用例使用 `--failed-reward`。

本次 lineage 中被修改的每个 PC 按指数移动平均更新 priority：

```text
p_new = max(1e-6, (1 - alpha) * p_old + alpha * reward)
```

其中 `alpha` 来自 `--priority-alpha`。priority 保存在 state metadata 中，因此会随
checkpoint 一同恢复。

### Corpus 上界

`--max-corpus-size` 只约束通过用例。超过上限时，算法按用例到初始失败的组合距离升序
排列，保留最近的通过用例并淘汰较远者；初始失败用例不会被淘汰。

## 4.6 三种模式对比

| 模式 | Scheduler | Mutation 范围 | 自适应信息 | 典型用途 |
| --- | --- | --- | --- | --- |
| Random | 队列 | 整个 `.text` | 无 | 基线实验 |
| PSBFL | 队列 | 初始失败轨迹窗口 | 静态位置权重 | 面向失败附近快速采样 |
| WitHW | 距离加权 | 失败轨迹尾部窗口 | PC priority、reward、受限 corpus | 持续搜索近失败且多样的通过用例 |

模式参数必须放在 mode 名称之后。完整命令布局见[第 2 章](02-cli-and-workflows.md)。
