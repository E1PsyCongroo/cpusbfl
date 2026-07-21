# 第 5 章：距离、用例选择与故障定位

analysis 阶段从 corpus 中选择通过用例，构造覆盖频谱，计算 SBFL 可疑度，并可选地把
覆盖点映射为 RTL block。generation 正常结束后也会调用同一套分析流程。

## 5.1 覆盖率距离

对于同名 coverage observer，先对计数做 `ln(x + 1)` 变换，再计算归一化欧氏距离：

```text
d_c(a, b) = sqrt(sum_i((ln(a_i + 1) - ln(b_i + 1))^2)) / sqrt(point_count)
```

存在多种 coverage 时，对各 coverage 的距离取平均。对数变换降低了高执行次数覆盖点
对距离的支配程度。

## 5.2 状态距离

状态距离当前使用整数寄存器和 CSR tracker，不直接把 PC tracker 纳入距离。两个状态
序列通过 FastDTW（radius = 1）对齐，再以最短路径的平均 cost 作为距离。

- 整数寄存器状态：不同寄存器数量除以 32；
- CSR 状态：当前实现比较 17 个字段，并以 17 归一化；
- 单个 core state：对寄存器距离和 CSR 距离取平均；
- 序列距离：DTW path 总 cost 除以 path 长度。

PC 仍用于轨迹窗口、mutation 候选和覆盖率约简，只是不直接进入上述状态距离。

## 5.3 分位数归一化与组合距离

覆盖率距离和状态距离的数值尺度不同。需要合并时，代码分别对一组距离做 rank-based
quantile transform：相同值使用平均 rank，原始距离为 0 时保持 0，最终映射到
`[0, 1]`。

```text
combined = cover_weight * normalized_coverage
         + (1 - cover_weight) * normalized_state
```

注意，归一化依赖当前参与比较的用例集合；同一对用例放入不同 corpus 时，其归一化后
的组合距离可能不同。

## 5.4 通过用例过滤

选择器只考虑满足以下条件的 corpus entry：

- `PassedMetadata.is_passed == true`；
- tracker 数据非空。

每个候选首先计算到初始失败用例的原始覆盖率和状态距离，再分别做分位数归一化并组合
为 `fail_distance`。

## 5.5 Selection 策略

### Random

随机打乱候选，取前 `--top-pass` 个。该模式不保证靠近初始失败，也不显式保证用例间
多样性。

### Sort

按 `fail_distance` 升序选择，即优先选择最接近初始失败的通过用例。距离相同时按
corpus ID 升序保持结果确定性。

### Diverse

Diverse 在“接近失败”和“已选通过用例之间保持差异”之间折中：

1. 按 `fail_distance` 升序形成候选池；
2. pool 大小为 `min(total, max(limit, limit * pool_factor))`；
3. 第一项固定选择最接近初始失败的通过用例；
4. 预先计算候选池中所有 pass-pass pair 的覆盖率与状态距离，并分别做分位数归一化；
5. 对每个待选通过用例 `i`，计算它到**已经选中的通过用例**的最小组合距离 `m_i`；
6. 选择 score 最大的用例：

```text
score_i = (1 - lambda) * (1 - fail_distance_i) + lambda * m_i
```

这里 `m_i` 就是 `min_pass_distance`。它不应计算为待选用例到失败用例的距离；失败距离
已经由 `fail_distance_i` 单独表达。第一项没有已选 pass 可比较，因此结果中使用 0
作为 sentinel。

score 相同时，依次优先：更小的失败距离、更大的多样性距离、更小的 corpus ID。

## 5.6 覆盖率约简

SBFL 希望保留与失败或修复直接相关的覆盖差异。分析前可对覆盖率做两类约简。

### 初始失败用例

若失败轨迹最后一个 PC 只出现一次，代码将最后一条指令替换为 NOP，并少执行一条指令：

- 修改后仍失败：直接使用该次运行的覆盖率；
- 修改后通过：从原覆盖计数中逐点饱和减去该次运行的覆盖率。

### 通过用例

通过用例约简依赖 `MutationMetadata`。代码寻找与失败轨迹的公共前缀，直到遇到首个轨迹
不一致或被 mutation 的 PC，再尝试 NOP 后缀指令。若约简后的前缀仍通过，则从该用例
覆盖率中减去前缀覆盖；若变为失败，则回退约简范围。

## 5.7 频谱矩阵

每个覆盖点统计四个值：

| 符号 | 含义 |
| --- | --- |
| `ef` | 执行该点的失败用例数 |
| `ep` | 执行该点的通过用例数 |
| `nf` | 未执行该点的失败用例数 |
| `np` | 未执行该点的通过用例数 |

当前工作流通常只有一个初始失败用例，以及至多 `--top-pass` 个选中的通过用例。

## 5.8 SBFL 指标

令 `F = ef + nf`，`P = ep + np`，支持的可疑度公式如下：

| 指标 | 当前实现 |
| --- | --- |
| Tarantula | `(ef/F) / (ef/F + ep/P)` |
| Ochiai | `ef / sqrt(F * (ef + ep))` |
| Jaccard | `ef / (ef + nf + ep)` |
| DStar | `ef^2 / (ep + nf)` |
| GP19 | `ef * (1 + 1 / (2*ep + ef))` |
| Barinel | `ef / (ef + ep)` |
| Crosstab | `abs(ef - expected) / sqrt(expected)`，其中 `expected=(ef+ep)*F/(F+P)` |
| Zoltar | `ef / (ef + nf + ep + 10000*nf*ep/ef)` |
| Ample | `abs(ef/F - ep/P)` |

实现对零分母分别做了保护；例如 DStar 在 `ef > 0` 且分母为零时返回正无穷。
所有 coverage observer 的覆盖点合并后按可疑度降序写入 `result.log`。

## 5.9 RTL block 映射

设置 `--rtl <DIR>` 后，代码读取该目录下直接存在的 `.sv` 和 `.v` 文件，不递归扫描
子目录。解析器从 module 实例关系和 generate block 标识构造层次，再识别：

- combinational/sequential `always` block；
- continuous assignment；
- module input/output（用于结构信息，不进入最终排名）。

覆盖点通过文件、层次和行号映射到 block。`blocks.json` 保存解析结果；`result.log` 中的
block 排名对同一 block 去重，保留其最高可疑覆盖点所对应的得分。

RTL 源文件、Verilator coverage 中的文件名/层次和构建时使用的源码必须一致，否则覆盖
点可能无法映射。映射失败不会改变覆盖点本身的 SBFL 排名。

