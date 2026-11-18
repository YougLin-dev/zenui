---
doc-id: ADR-0019
layer: decisions
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/spec/02-runtime.md
  - docs/spec/08-gates.md
  - docs/decisions/adr-0017-p2-runtime-authority-and-transaction-shape.md
  - docs/decisions/adr-0011-docs-four-layers.md
replaces: []
---

# ADR-0019：失败分类按检测点，阶段违规以值错误报告并毒化事务

## 背景

ADR-0017 把事务错误分成「值错误」与「致命错误」两档，并规定 phase violation 属致命。P2 实现时出现两处歧义，都会让同一份裁决读出两种实现：

1. **`SPEC-RT-053` 的括号列表被读成了错误类别。** 原文是「任一 preflight 失败（duplicate key、`InvalidGeneration`、`InvalidHandle`、read failure、phase violation）MUST 丢弃本事务的全部 scratch」。但 `InvalidHandle` 也可能来自一次**写 API 调用**——那时操作还没有进入 journal，没有任何 scratch 可丢；按「错误类别」读，会把一个被拒绝的写升级成整帧回滚，而 A0-04 的 step 08 恰好要求相反：被拒绝的写，帧照常提交（`commits=1`、`io_writes=0`）。
2. **阶段违规的报告点与致命性错位。** 写 API 对调用方只暴露一种错误类型（否则每个调用点都要 match 两层错误）。可阶段违规发生在写 API 内部，而它的后果是「整帧不可提交」。若让写 API 直接返回致命错误，一个操作的错误就会通过 `?` 无声地中止一个本来无关的帧；若让它返回普通值错误而不毒化，事务会带着一次非法写继续走到 commit。

这两处都不是措辞偏好：一处决定已验证的 golden 语义能不能保住，另一处决定 `SPEC-RT-055` 的「失败后可恢复」在类型上如何表达。

## 备选项

1. **按错误类别分类**：`InvalidHandle` 永远是值错误、phase violation 永远是致命错误。否决：同一个 `InvalidHandle` 在写 API 与 preflight 两个检测点后果不同；按类别分类会让 preflight 里的陈旧句柄无法整帧丢弃。
2. **写 API 直接返回致命错误**（phase violation 返回 `Fatal`）。否决：写 API 出现两种错误类型，每个调用点都要分别处理；`?` 会把单次操作的失败升级为整帧中止，与 `SPEC-RT-057` 的值语义冲突。
3. **外部 phase guard**：调用写 API 之前由调用方检查阶段。否决：重复实现且覆盖不到嵌套调用与内部 privileged 路径；只有写 API 自己知道本次写属于哪种 `WriteKind`。
4. **独立的 `WriteError::{Value, Phase}` 两层枚举**。否决：每个调用点都要两层 match，收益为零；失败分类已经是「是否毒化」这一个布尔维度。
5. **检测点规则 + 值通道报告、毒化承载致命性**（选中）：写 API 只有一种错误类型；pre-journal 拒绝与 preflight 检测按**发生位置**分档；阶段违规用值错误返回，同时把事务置为中毒，由最外层 exit 兑现致命后果。

## 决定

1. **失败按检测点分类，不按错误载荷分类。** 在操作进入 journal **之前**被写 API 拒绝的输入（坏 handle、非排列的 `reorder` 参数、未知 key）是**值错误**：操作从未暂存，错误按 `SPEC-RT-057` 进入 sink，帧继续提交，`io_writes` 不增。只有 preflight/commit 在读取 journal 与权威状态时发现的**不一致**才是致命错误。
2. **`SPEC-RT-053` 的括号列表是检测点示例**，不是错误类别清单；该条以「在 preflight 阶段检测到的」限定。同一种错误载荷在两个检测点可以有不同后果。
3. **写 API 的阶段违规返回 `ValueError::Phase(PhaseViolation)`**，其中 `PhaseViolation` 携带 `{ phase, attempted }`。返回值本身是值错误，但它**毒化**事务。
4. **`commit` 阶段的违规返回 `Fatal::PhaseViolation(PhaseViolation)`**，不存在值通道；提交阶段不执行用户回调（`SPEC-RT-054`），调用方不在这里接收错误。
5. **`PhaseViolation` 是同一个结构，两个通道各包装一次**，不允许在两个枚举里各定义一份字段，否则两处会漂移。
6. **毒化后的行为**：后续写调用返回 `ValueError::Aborted`；帧体即使返回 `Ok`，最外层也必须返回已锁存的 `Fatal`；`commits` 不增；stage 丢弃；下一帧在 phase/depth 复位后可正常提交（`SPEC-RT-055`）。
7. **与 ADR-0017 决定 7 的显式错位**：决定 7 把 phase violation 归入致命错误。本 ADR 只在**行为**上维持该结论（帧不可提交），在**类型**上第一次报告走的是 `ValueError`。因此「致命 = `Fatal`」不是一一对应，任何文档、测试名或评审结论都不得写成「`ValueError` 永不中止帧」。ADR-0017 不就地修订，本 ADR 是其细化记录。
8. **明确不做**：写 API 直接返回 `Fatal`；外部 phase guard；`From<ValueError> for Fatal`（会让 `?` 隐式升级）；独立的两层 `WriteError`。

## 依据

- `SPEC-RT-053`：要求的是「preflight 失败 → 整帧丢弃、状态与上次提交完全一致」，其主语是 preflight 阶段的失败，而非任意来源的同名错误载荷。
- `SPEC-RT-038`：阶段违规 MUST 以 typed 领域错误返回，MUST NOT 让 `RefCell` 借用文本泄漏、MUST NOT panic；错误类型属于哪一层由实现决定，但必须是可模式匹配的类型。
- `SPEC-RT-057`：读取、binding、事件写入、mount 的错误进入同一 sink，MUST NOT 静默替换旧值；写 API 的拒绝必须是可观测的值。
- `SPEC-RT-033`：只有最外层 exit 提交一次；毒化在 exit 兑现，正是该条的唯一提交点。
- `SPEC-RT-055`：失败后必须恢复，下一次干净事务仍可提交；毒化必须在这一帧结束时清掉。
- `SPEC-GATE-A0-04`：被拒绝的写不改变提交次数与 `io_writes`；`SPEC-GATE-A0-05`：`WriteDuringCompute` 是可观测的领域错误且可恢复；`SPEC-GATE-A0-06`：preflight 注入失败后整帧丢弃、随后一笔干净事务可提交。

## 影响与撤销条件

影响：

- 写 API 统一为 `Result<T, ValueError>`；`ValueError` 至少含 `Handle`、`Store`、`Phase(PhaseViolation)`、`Aborted` 四项。
- 帧的 `Result` 可以因锁存的致命错误而为 `Err`，即使帧体返回了 `Ok`；夹具必须覆盖这条，不能只断言帧体的返回值。
- A0-05 的夹具要同时断言三件事：写调用得到 `Phase` 值错误、帧的 `commits` 不前进、下一帧提交成功。
- `ValueError` 不再等价于「无害」：文档与测试不得用「值错误不会中止帧」作为归纳。
- 文档层不改写任何历史 ADR；`SPEC-RT-053` 只加限定词与澄清，不改 MUST 强度。

撤销条件（可判定）：

- 若新增一个**不属于任何帧**的写 API（例如直接模型命令），毒化没有可兑现的最外层 exit，则该 API 必须直接返回致命错误；出现这种 API 时新开 ADR。
- 若出现一个夹具，要求调用方在同一帧内区分「本次调用违规」与「事务已被更早的违规毒化」并能据此继续，而当前形状无法表达，则改为两层错误通道，并新开 ADR。
